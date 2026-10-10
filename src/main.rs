//! rtld —— BORUIX 用户态动态链接器（阶段 5 / 3P5-1）。
//!
//! ## 架构决策：`ld.so` 模型
//!
//! 本系统的 `exec` 是**用户态读好镜像再交给内核**的（`Image::Slice`/`Image::Shm`），内核
//! **没有**"按路径读程序"的能力；而标准 `PT_INTERP` 自动装载要求内核按路径读解释器。
//! 故采用 `rtld <program> [args...]`：rtld 自己读文件、自己 mmap、自己做重定位。
//! **内核不需要理解 ELF 重定位**（S13 边界）。
//!
//! ## 能力
//!
//! 1. 解析 ELF64 头/程序头/`PT_DYNAMIC`；
//! 2. 一次 mmap 覆盖全部 `PT_LOAD` 的跨度（保证段间相对偏移不变）+ 拷贝 + 补零 bss + 逐段 mprotect；
//! 3. **`DT_NEEDED` 递归装载**（在程序所在目录与 `/lib` 中查找）；
//! 4. **跨模块符号解析**（主对象优先，其后按装载顺序）；
//! 5. 重定位：`R_X86_64_RELATIVE` / `GLOB_DAT` / `JUMP_SLOT`；
//! 6. 调用导出符号（验收用）。
//!
//! **尚未做（如实声明）**：跳到 `e_entry`（现为调用导出函数）、`.so` 级 TLS（ADR-053）、
//! `dlopen`/`dlsym`（3P5-3）。遇到不支持的重定位类型**如实报错**，绝不静默跳过。

#![allow(dead_code)]

extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use boruix_std::{args, println};

const PAGE: u64 = 4096;

// ---------- ELF64 ----------

#[derive(Debug, Clone, Copy)]
struct ElfHeader {
    e_type: u16,
    e_machine: u16,
    e_entry: u64,
    e_phoff: u64,
    e_phentsize: u16,
    e_phnum: u16,
}

#[derive(Debug, Clone, Copy)]
struct Phdr {
    p_type: u32,
    p_flags: u32,
    p_offset: u64,
    p_vaddr: u64,
    p_filesz: u64,
    p_memsz: u64,
}

fn rd_u16(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}

fn rd_u32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

fn rd_u64(b: &[u8], off: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[off..off + 8]);
    u64::from_le_bytes(a)
}

fn parse_header(b: &[u8]) -> Result<ElfHeader, String> {
    if b.len() < 64 {
        return Err("文件小于 ELF64 头".to_string());
    }
    if &b[0..4] != b"\x7fELF" {
        return Err("不是 ELF（魔数不符）".to_string());
    }
    if b[4] != 2 || b[5] != 1 {
        return Err("不是小端 ELF64".to_string());
    }
    Ok(ElfHeader {
        e_type: rd_u16(b, 16),
        e_machine: rd_u16(b, 18),
        e_entry: rd_u64(b, 24),
        e_phoff: rd_u64(b, 32),
        e_phentsize: rd_u16(b, 54),
        e_phnum: rd_u16(b, 56),
    })
}

fn parse_phdrs(b: &[u8], h: &ElfHeader) -> Result<Vec<Phdr>, String> {
    if h.e_phentsize as usize != 56 {
        return Err("phentsize != 56".to_string());
    }
    let mut v = Vec::new();
    for i in 0..h.e_phnum as u64 {
        let off = (h.e_phoff + i * 56) as usize;
        if off + 56 > b.len() {
            return Err("程序头表越界".to_string());
        }
        v.push(Phdr {
            p_type: rd_u32(b, off),
            p_flags: rd_u32(b, off + 4),
            p_offset: rd_u64(b, off + 8),
            p_vaddr: rd_u64(b, off + 16),
            p_filesz: rd_u64(b, off + 32),
            p_memsz: rd_u64(b, off + 40),
        });
    }
    Ok(v)
}

fn align_down(v: u64) -> u64 {
    v & !(PAGE - 1)
}

fn align_up(v: u64) -> u64 {
    (v + PAGE - 1) & !(PAGE - 1)
}

fn prot_of(flags: u32) -> u64 {
    let mut p = 0u64;
    if flags & 4 != 0 {
        p |= 1;
    }
    if flags & 2 != 0 {
        p |= 2;
    }
    if flags & 1 != 0 {
        p |= 4;
    }
    p
}

// ---------- 动态节 ----------

const DT_NULL: u64 = 0;
const DT_NEEDED: u64 = 1;
const DT_PLTRELSZ: u64 = 2;
const DT_HASH: u64 = 4;
const DT_STRTAB: u64 = 5;
const DT_SYMTAB: u64 = 6;
const DT_RELA: u64 = 7;
const DT_RELASZ: u64 = 8;
const DT_RELAENT: u64 = 9;
const DT_STRSZ: u64 = 10;
const DT_SYMENT: u64 = 11;
const DT_JMPREL: u64 = 23;

const R_X86_64_GLOB_DAT: u32 = 6;
const R_X86_64_JUMP_SLOT: u32 = 7;
const R_X86_64_RELATIVE: u32 = 8;

/// 该 DT tag 的值是否为**虚拟地址**（装载后须加基址才能解引用）。
///
/// 非地址型（SIZE/ENT/NEEDED/COUNT/SONAME）是标量或字符串表偏移，**不得**加基址。
/// 这是 ELF 规范事实，集中在此单点（S15）。
fn is_dynamic_addr(tag: u64) -> bool {
    matches!(tag, 3 | 4 | 5 | 6 | 7 | 12 | 13 | 17 | 23 | 25 | 26 | 0x6ffffef5)
}

fn read_dynamic(dyn_va: u64, base: u64) -> Vec<(u64, u64)> {
    let mut v = Vec::new();
    let mut p = dyn_va;
    loop {
        let tag = unsafe { *(p as *const u64) };
        let raw = unsafe { *((p + 8) as *const u64) };
        let val = if is_dynamic_addr(tag) {
            base.wrapping_add(raw)
        } else {
            raw
        };
        v.push((tag, val));
        if tag == DT_NULL {
            break;
        }
        p += 16;
    }
    v
}

fn dyn_get(d: &[(u64, u64)], tag: u64) -> Option<u64> {
    d.iter().find(|(t, _)| *t == tag).map(|(_, v)| *v)
}

/// 读 C 字符串（NUL 结尾）。
unsafe fn read_cstr(mut p: u64) -> String {
    let mut s = String::new();
    loop {
        let c = unsafe { *(p as *const u8) };
        if c == 0 {
            break;
        }
        s.push(c as char);
        p += 1;
        if s.len() > 4096 {
            break; // 防御：畸形镜像不该让我们无限读下去
        }
    }
    s
}

// ---------- 已装载对象 ----------

struct Object {
    path: String,
    base: u64,
    /// 整段映射的字节数（与 `mmap` 请求的 span 成对）。
    ///
    /// 必须在装载时记下：span 由 ELF 程序头算出，`dlclose` 时镜像早已不在手里，
    /// 事后无法再推；而 `munmap` 必须与 `mmap` 的地址与长度**严格成对**。
    map_len: u64,
    dyn_: Vec<(u64, u64)>,
    /// 该对象是否带 `PT_TLS`。
    ///
    /// 用于 ADR-053 的 **D4**：`dlopen` 一个带 TLS 的 `.so` 必须**如实拒绝**——
    /// initial-exec 的偏移必须在进程启动时确定，运行期追加会改变已有模块的偏移
    /// （已装载模块的 GOT 槽全部失效）。
    has_tls: bool,
    /// dlopen 句柄 id（稳定标识，不随对象表增删而变）。
    ///
    /// 为什么不是「表下标 + 1」：dlclose 会把对象移出表，下标随之漂移；用稳定 id 才能让
    /// 旧句柄**明确失效**（查不到 ⇒ 报错），而不是**悄悄指向另一个对象**。
    handle: u64,
    /// 引用计数。
    ///
    /// - 大于 0：由 dlopen 装载且仍被引用；归零即摘除；
    /// - -1：**启动期**装载（主程序与其 DT_NEEDED 闭包）——永不卸载，
    ///   对它们的 dlclose 是合法的空操作。
    refs: i32,
    /// RTLD_NODELETE：置位时引用计数归零也**不摘除**对象。
    nodelete: bool,
}

impl Object {
    /// 该对象的 `.dynsym` 符号总数（经 DT_HASH 的 nchain）。
    fn nsyms(&self) -> usize {
        match dyn_get(&self.dyn_, DT_HASH) {
            Some(h) => unsafe { *((h + 4) as *const u32) as usize },
            None => 0,
        }
    }

    /// 取第 i 个动态符号的 (名字, st_value, st_shndx)。
    fn sym(&self, i: usize) -> Option<(String, u64, u16)> {
        let symtab = dyn_get(&self.dyn_, DT_SYMTAB)?;
        let strtab = dyn_get(&self.dyn_, DT_STRTAB)?;
        let s = symtab + (i as u64) * 24;
        let st_name = unsafe { *(s as *const u32) } as u64;
        let st_value = unsafe { *((s + 8) as *const u64) };
        let st_shndx = unsafe { *((s + 6) as *const u16) };
        let name = if st_name == 0 {
            String::new()
        } else {
            unsafe { read_cstr(strtab + st_name) }
        };
        Some((name, st_value, st_shndx))
    }
}

// ---------- 装载 ----------

fn load_object(image: &[u8], path: &str) -> Result<Object, String> {
    let h = parse_header(image)?;
    if h.e_machine != 62 {
        return Err("不是 x86-64".to_string());
    }
    let phdrs = parse_phdrs(image, &h)?;
    let loads: Vec<&Phdr> = phdrs.iter().filter(|p| p.p_type == 1).collect();
    if loads.is_empty() {
        return Err("没有 PT_LOAD".to_string());
    }
    let min_va = loads.iter().map(|p| align_down(p.p_vaddr)).min().unwrap();
    let max_va = loads.iter().map(|p| align_up(p.p_vaddr + p.p_memsz)).max().unwrap();
    // 页对齐后再作为请求长度：munmap 要求长度是 4KiB 整数倍，故这里算出的值
    // 必须同时用于 mmap 与 munmap，不能只在装载时算一次、卸载时另算一次。
    let span = align_up(max_va - min_va);
    let base = match boruix_std::libsys::mmap_prot(span, 1 | 2) {
        Ok(a) => a,
        Err(e) => return Err(alloc::format!("mmap({}) 失败: {:?}", span, e)),
    };
    for p in &loads {
        let dst = base + (p.p_vaddr - min_va);
        let src_off = p.p_offset as usize;
        let filesz = p.p_filesz as usize;
        if src_off + filesz > image.len() {
            return Err("段内容越界".to_string());
        }
        unsafe {
            core::ptr::copy_nonoverlapping(image.as_ptr().add(src_off), dst as *mut u8, filesz);
            let bss = (p.p_memsz - p.p_filesz) as usize;
            if bss > 0 {
                core::ptr::write_bytes((dst + p.p_filesz) as *mut u8, 0, bss);
            }
        }
    }
    for p in &loads {
        let start = align_down(base + (p.p_vaddr - min_va));
        let end = align_up(base + (p.p_vaddr - min_va) + p.p_memsz);
        if let Err(e) = boruix_std::libsys::mprotect(start, end - start, prot_of(p.p_flags)) {
            return Err(alloc::format!("mprotect(0x{:x}) 失败: {:?}", start, e));
        }
    }
    let dyn_va = phdrs
        .iter()
        .find(|p| p.p_type == 2)
        .map(|p| base + (p.p_vaddr - min_va));
    let dyn_ = match dyn_va {
        Some(v) => read_dynamic(v, base),
        None => Vec::new(),
    };
    Ok(Object {
        path: path.to_string(),
        base,
        map_len: span,
        dyn_,
        has_tls: phdrs.iter().any(|p| p.p_type == 7),
        // 句柄与引用计数由调用方决定：启动期装载给 -1（永久），dlopen 给 1。
        handle: 0,
        refs: 0,
        nodelete: false,
    })
}

/// 在目录 `dir` 与 `/lib` 中查找共享库。
fn find_library(name: &str, dir: &str) -> Option<String> {
    let c1 = alloc::format!("{}/{}", dir, name);
    if boruix_std::libsys::stat(&c1).is_ok() {
        return Some(c1);
    }
    let c2 = alloc::format!("/lib/{}", name);
    if boruix_std::libsys::stat(&c2).is_ok() {
        return Some(c2);
    }
    None
}

/// 递归装载（广度优先）：返回全部对象，第 0 个是主对象。
fn load_closure(main_path: &str) -> Result<Vec<Object>, String> {
    let dir = match main_path.rfind('/') {
        Some(i) => &main_path[..i],
        None => ".",
    };
    let mut objs: Vec<Object> = Vec::new();
    let image = boruix_std::fs::read_to_end(main_path)
        .map_err(|e| alloc::format!("读取 {} 失败: {:?}", main_path, e))?;
    objs.push(load_object(&image, main_path)?);
    let mut i = 0usize;
    while i < objs.len() {
        // 收集本对象的 DT_NEEDED（值是指向 STRTAB 的偏移，故须经该对象的 STRTAB 解读）。
        let mut wanted: Vec<String> = Vec::new();
        let strtab = dyn_get(&objs[i].dyn_, DT_STRTAB);
        for (t, v) in &objs[i].dyn_ {
            if *t == DT_NEEDED {
                if let Some(st) = strtab {
                    wanted.push(unsafe { read_cstr(st + *v) });
                }
            }
        }
        for name in wanted {
            if objs.iter().any(|o| o.path.ends_with(&name)) {
                continue; // 已装载（按文件名去重，够本阶段用）
            }
            let Some(full) = find_library(&name, dir) else {
                return Err(alloc::format!("找不到依赖库 {}", name));
            };
            println!("[rtld] 装载依赖 {} -> {}", name, full);
            let img = boruix_std::libsys::read_to_end(&full)
                .map_err(|e| alloc::format!("读取 {} 失败: {:?}", full, e))?;
            objs.push(load_object(&img, &full)?);
        }
        i += 1;
    }
    Ok(objs)
}

/// 全局作用域符号解析：返回**绝对地址**（主对象优先，其后按装载顺序）。
///
/// `st_shndx == 0` 表示**未定义**（引用），不算定义——这一条是正确性的关键：
/// 若不排除，引用方自己的 UND 符号会先被命中，拿到的是 0 或错值。
fn resolve(objs: &[Object], name: &str) -> Option<u64> {
    // **内置表优先**：`dlopen`/`dlsym`/`dlerror` 由 rtld 自己提供（3P5-3）。
    if let Some(a) = builtin_symbol(name) {
        return Some(a);
    }
    for o in objs.iter() {
        for si in 1..o.nsyms() {
            if let Some((n, val, shndx)) = o.sym(si) {
                if shndx != 0 && n == name {
                    return Some(o.base + val);
                }
            }
        }
    }
    None
}

/// 应用一段重定位表（.rela.dyn 或 .rela.plt）。
fn apply_range(obj: &Object, objs: &[Object], rela: u64, relasz: u64) -> Result<usize, String> {
    let relaent = dyn_get(&obj.dyn_, DT_RELAENT).unwrap_or(24);
    if relaent != 24 {
        return Err("RELAENT != 24".to_string());
    }
    let symtab = dyn_get(&obj.dyn_, DT_SYMTAB).unwrap_or(0);
    let strtab = dyn_get(&obj.dyn_, DT_STRTAB).unwrap_or(0);
    let mut n = 0usize;
    let mut off = 0u64;
    while off + 24 <= relasz {
        let e = rela + off;
        let r_offset = unsafe { *(e as *const u64) };
        let r_info = unsafe { *((e + 8) as *const u64) };
        let r_addend = unsafe { *((e + 16) as *const u64) };
        let r_type = (r_info & 0xffff_ffff) as u32;
        let r_sym = (r_info >> 32) as usize;
        match r_type {
            R_X86_64_RELATIVE => {
                unsafe { *((obj.base + r_offset) as *mut u64) = obj.base.wrapping_add(r_addend) };
                n += 1;
            }
            R_X86_64_GLOB_DAT | R_X86_64_JUMP_SLOT => {
                // 取符号名 → 在全局作用域里找定义 → 写实际地址。
                let s = symtab + (r_sym as u64) * 24;
                let st_name = unsafe { *(s as *const u32) } as u64;
                let name = unsafe { read_cstr(strtab + st_name) };
                let Some(addr) = resolve(objs, &name) else {
                    return Err(alloc::format!("未定义符号 {}", name));
                };
                unsafe { *((obj.base + r_offset) as *mut u64) = addr };
                n += 1;
            }
            other => {
                return Err(alloc::format!(
                    "不支持的重定位类型 {} @ 0x{:x}（未静默跳过）",
                    other, r_offset
                ));
            }
        }
        off += 24;
    }
    Ok(n)
}

fn relocate_all(objs: &[Object]) -> Result<usize, String> {
    let mut total = 0usize;
    for o in objs {
        if let (Some(rela), Some(sz)) = (dyn_get(&o.dyn_, DT_RELA), dyn_get(&o.dyn_, DT_RELASZ)) {
            total += apply_range(o, objs, rela, sz)?;
        }
        if let (Some(jmprel), Some(sz)) =
            (dyn_get(&o.dyn_, DT_JMPREL), dyn_get(&o.dyn_, DT_PLTRELSZ))
        {
            total += apply_range(o, objs, jmprel, sz)?;
        }
    }
    Ok(total)
}

// ---------- 运行期装载 API（3P5-3：dlopen/dlsym）----------
//
// 由 **rtld 自己**提供（不是 libc）：这些调用需要 rtld 的装载器状态（已装载对象表）。
// 被装载程序里的 `UND dlopen` 经 `resolve` 的**内置表**解析到这里。

/// 已装载对象表。
///
/// **前提（如实声明）**：rtld 目前单线程运行，故不加锁。若将来 rtld 引入并发装载，
/// 这里必须换成带锁的容器。
static mut OBJECTS: Vec<Object> = Vec::new();
/// 最近一次错误的描述（`dlerror` 的返回值）。
static mut LAST_ERR: Option<String> = None;
/// dlerror 返回的 **C 串缓冲**（末尾带 NUL）。
///
/// **为什么必须有它**：Rust 的 String / Vec<u8> **不保证**末尾有 NUL，把
/// String::as_ptr() 直接交给 C 的 %s 会**读越界**。机内实测（2026-10）：
/// err=dlclose: 句柄非法3p/libadd.s ——错误串后面粘着无关的路径片段，
/// 正是「缓冲区后面恰好跟着别的数据」。故这里显式补 NUL 再返回。
static mut LAST_ERR_C: Vec<u8> = Vec::new();

fn set_err(msg: &str) {
    unsafe { LAST_ERR = Some(msg.to_string()) };
}

/// dlopen 旗标（取值与 glibc 一致；编译期常量，无 ABI 含义）。
const RTLD_LAZY: i32 = 0x00001;
const RTLD_NOW: i32 = 0x00002;
const RTLD_NOLOAD: i32 = 0x00004;
const RTLD_GLOBAL: i32 = 0x00100;
const RTLD_NODELETE: i32 = 0x01000;

/// 句柄 id 分配器（从 1 起；0 保留为「空句柄」）。
static mut NEXT_HANDLE: u64 = 1;

fn next_handle() -> u64 {
    unsafe {
        let p = core::ptr::addr_of_mut!(NEXT_HANDLE);
        let h = *p;
        *p = h + 1;
        h
    }
}

/// `dlopen(path, flags)`：运行期装载一个共享库，返回**不透明句柄**（失败返回 NULL）。
///
/// **支持**：`RTLD_LAZY`/`RTLD_NOW`/`RTLD_GLOBAL`/`RTLD_LOCAL` 的语义（本实现一律**立即**
/// 解析全部重定位，故 LAZY 与 NOW 无区别——如实声明）；`RTLD_NOLOAD` 与
/// `RTLD_NODELETE` 均已实现。
///
/// **未做（如实声明）**：本函数**不**自动装载该对象的 `DT_NEEDED`——依赖库必须由调用方
/// 自己先 `dlopen`（`load_closure` 只用于启动期闭包）。因此 `dlclose` 的「仍有依赖者
/// 则拒绝卸载」判定，依据的是调用方**实际装载过**的那些对象所声明的 `DT_NEEDED`。
///
/// **带 `PT_TLS` 的 `.so` 如实拒绝**（ADR-053 D4）：initial-exec 的偏移必须在进程启动时
/// 确定，运行期追加会改变已有模块的偏移，已装载模块的 GOT 槽会全部失效。
#[unsafe(no_mangle)]
pub extern "C" fn dlopen(path: *const u8, flags: i32) -> *mut u8 {
    if path.is_null() {
        set_err("dlopen: path 为 NULL");
        return core::ptr::null_mut();
    }
    let p = unsafe { read_cstr(path as u64) };
    // 未知旗标**如实拒绝**（S09：静默忽略等于给出错误的成功）。已知集合见上方常量；
    // RTLD_LAZY 与 RTLD_NOW 在本实现里无区别（一律立即解析全部重定位——如实声明）。
    const KNOWN: i32 = RTLD_LAZY | RTLD_NOW | RTLD_NOLOAD | RTLD_GLOBAL | RTLD_NODELETE;
    if (flags & !KNOWN) != 0 {
        set_err(&alloc::format!("dlopen: 不支持的旗标 0x{:x}", flags & !KNOWN));
        return core::ptr::null_mut();
    }
    let noload = (flags & RTLD_NOLOAD) != 0;
    // **先查已装载表**（按路径精确匹配；不做 realpath / 符号链接解析——如实声明）：
    //   - RTLD_NOLOAD：命中即返回**同一句柄**；未命中**不装载**，返回 NULL；
    //   - 普通 dlopen：命中即**递增引用计数**并返回同一句柄。
    // 此前每次调用都 push 一份新对象 ⇒ 同一个 .so 会有两份映射、两个互不相等的句柄。
    {
        let objs = unsafe { &mut *core::ptr::addr_of_mut!(OBJECTS) };
        if let Some(o) = objs.iter_mut().find(|o| o.path == p) {
            if o.refs >= 0 {
                o.refs += 1;
            }
            let h = o.handle;
            unsafe { LAST_ERR = None };
            return h as *mut u8;
        }
    }
    if noload {
        set_err(&alloc::format!("dlopen: {} 未装载（RTLD_NOLOAD 不装载）", p));
        return core::ptr::null_mut();
    }
    let img = match boruix_std::fs::read_to_end(&p) {
        Ok(v) => v,
        Err(_) => {
            set_err(&alloc::format!("dlopen: 读不到 {}", p));
            return core::ptr::null_mut();
        }
    };
    let mut o = match load_object(&img, &p) {
        Ok(o) => o,
        Err(e) => {
            set_err(&alloc::format!("dlopen: 装载 {} 失败: {}", p, e));
            return core::ptr::null_mut();
        }
    };
    if o.has_tls {
        // load_object 已经 mmap 过了。拒绝装载就必须把这段映射还回去——否则
        // 每次拒绝都泄漏一段地址空间，且调用方只看到 NULL，无从发现（S18）。
        let _ = boruix_std::libsys::munmap(o.base, o.map_len);
        set_err("dlopen: 该 .so 带 PT_TLS，运行期装载不支持（ADR-053 D4）");
        return core::ptr::null_mut();
    }
    o.refs = 1;
    o.nodelete = (flags & RTLD_NODELETE) != 0;
    o.handle = next_handle();
    let handle = o.handle;
    // 经裸指针访问 static mut：Rust 2024 禁止直接对 static mut 取引用（static_mut_refs）。
    unsafe { (*core::ptr::addr_of_mut!(OBJECTS)).push(o) };
    // 新对象的符号解析在**加入全局表之后**做：这样它能看见全局作用域，也能被看见。
    // （对已装载对象重复解析是**幂等**的：RELATIVE/GLOB_DAT/JUMP_SLOT 都写入同一值。）
    let objs = unsafe { &*core::ptr::addr_of!(OBJECTS) };
    if let Err(e) = relocate_all(objs) {
        // 失败即回滚：对象已入表且映射已建立。若只返回 NULL 而不清理，就会留下
        // 一个「表里有、句柄无人知」的对象——地址空间泄漏，且后续对同一路径的
        // dlopen 会**命中它并返回一个从未成功装载过的句柄**。
        let objs = unsafe { &mut *core::ptr::addr_of_mut!(OBJECTS) };
        if let Some(i) = objs.iter().position(|o| o.handle == handle) {
            let base = objs[i].base;
            let len = objs[i].map_len;
            objs.remove(i);
            let _ = boruix_std::libsys::munmap(base, len);
        }
        set_err(&alloc::format!("dlopen: {} 重定位失败: {}", p, e));
        return core::ptr::null_mut();
    }
    unsafe { LAST_ERR = None };
    handle as *mut u8
}

/// `dlsym(handle, name)`：在指定句柄对应的对象里查符号（找不到返回 NULL）。
#[unsafe(no_mangle)]
pub extern "C" fn dlsym(handle: *mut u8, name: *const u8) -> *mut u8 {
    let h = handle as u64;
    if h == 0 || name.is_null() {
        set_err("dlsym: 句柄或名字非法");
        return core::ptr::null_mut();
    }
    let n = unsafe { read_cstr(name as u64) };
    let objs = unsafe { &*core::ptr::addr_of!(OBJECTS) };
    let Some(o) = objs.iter().find(|o| o.handle == h) else {
        set_err("dlsym: 未知句柄（可能已被 dlclose 摘除）");
        return core::ptr::null_mut();
    };
    for si in 1..o.nsyms() {
        if let Some((sn, val, shndx)) = o.sym(si) {
            if shndx != 0 && sn == n {
                return (o.base + val) as *mut u8;
            }
        }
    }
    set_err(&alloc::format!("dlsym: 未找到 {}", n));
    core::ptr::null_mut()
}

/// 取路径的最后一段。`DT_NEEDED` 里存的是裸库名（`libadd.so`），与装载时的全路径
/// 只能按尾段精确比对——用 `ends_with` 会把 `add.so` 误判成 `libadd.so` 的依赖。
fn basename(path: &str) -> &str {
    match path.rfind('/') {
        Some(i) => &path[i + 1..],
        None => path,
    }
}

/// 在世对象里，第一个把 `objs[idx]` 声明为 `DT_NEEDED` 的对象路径（没有则 `None`）。
///
/// 为什么这道门不可省：`dlclose` 现在会把该对象的整段映射归还。若还有别的在世
/// 对象依赖它，那些对象里**已经写好**的 GOT/PLT 槽就指向已归还的物理帧——帧一旦被
/// 复用，就是静默野写。这与「符号查找失败」不同：后者可诊断，前者不可。故此处
/// **如实拒绝**卸载并指明是谁依赖，而不是把不可诊断的破坏留给运行期。
fn dependent_on(objs: &[Object], idx: usize) -> Option<String> {
    let target = basename(&objs[idx].path).to_string();
    for (i, o) in objs.iter().enumerate() {
        if i == idx {
            continue;
        }
        let strtab = dyn_get(&o.dyn_, DT_STRTAB);
        for (t, v) in &o.dyn_ {
            if *t != DT_NEEDED {
                continue;
            }
            if let Some(st) = strtab {
                let dep = unsafe { read_cstr(st + *v) };
                if dep == target {
                    return Some(o.path.clone());
                }
            }
        }
    }
    None
}

/// dlclose(handle)：递减引用计数；**最后一个引用**释放时**归还地址空间**并摘除对象。
///
/// ## 真回收（本项修掉的正是这里的旧行为）
///
/// 旧实现在引用归零时只把对象移出符号解析作用域，**不 munmap**。后果是地址空间
/// 只增不减：反复 dlopen/dlclose 同一个 .so 会持续消耗虚存与物理帧。
/// 现在归零且未置 `RTLD_NODELETE` 时执行：
///   1. **依赖者检查**——若还有在世对象声明依赖它，**拒绝**卸载（返回 -1 并说明
///      是谁依赖）。这不是保守，而是唯一安全的选择，理由见 `dependent_on`。
///   2. `munmap(base, map_len)`——与装载时的 `mmap` 严格成对；长度取自
///      `Object.map_len`，不重算。
///   3. 移出对象表。
///
/// **任何一步失败都复原引用计数**：否则对象会停在「表里有、计数为 0」，既不会被
/// 再次回收，也不会被任何调用方持有——纯粹的泄漏。`munmap` 失败时对象**留在表内**
/// （先 munmap 成功再 remove，顺序不可颠倒，否则失败后对象已不可达却仍占着映射）。
///
/// ## 卸载之后
///
/// 映射已归还 ⇒ `RTLD_NOLOAD` 查不到它，且**已取出的函数指针再调用是野指针**
/// （POSIX 下这是调用方的责任；本实现不再提供「仍可调用」的宽松行为）。
///
/// 启动期装载的对象（主程序与其 DT_NEEDED 闭包）引用计数为 -1（永不卸载），
/// 对它们的 dlclose 是**合法的空操作**（返回 0）。
///
/// 返回值：0 = 成功；-1 = 句柄非法/未知/仍有依赖者/munmap 失败（同时置 dlerror）。
#[unsafe(no_mangle)]
pub extern "C" fn dlclose(handle: *mut u8) -> i32 {
    let h = handle as u64;
    if h == 0 {
        set_err("dlclose: 句柄非法");
        return -1;
    }
    let objs = unsafe { &mut *core::ptr::addr_of_mut!(OBJECTS) };
    let Some(idx) = objs.iter().position(|o| o.handle == h) else {
        set_err("dlclose: 未知句柄");
        return -1;
    };
    if objs[idx].refs < 0 {
        // 启动期对象：合法空操作。
        unsafe { LAST_ERR = None };
        return 0;
    }
    objs[idx].refs -= 1;
    if objs[idx].refs > 0 || objs[idx].nodelete {
        unsafe { LAST_ERR = None };
        return 0;
    }
    if let Some(dep) = dependent_on(objs, idx) {
        objs[idx].refs = 1;
        let msg = alloc::format!(
            "dlclose: {} 仍被 {} 依赖（须先卸载依赖者）",
            objs[idx].path,
            dep
        );
        set_err(&msg);
        return -1;
    }
    let base = objs[idx].base;
    let len = objs[idx].map_len;
    if let Err(e) = boruix_std::libsys::munmap(base, len) {
        objs[idx].refs = 1;
        let msg = alloc::format!(
            "dlclose: munmap(0x{:x}, {}) 失败: {:?}（对象仍留在表内）",
            base,
            len,
            e
        );
        set_err(&msg);
        return -1;
    }
    objs.remove(idx);
    unsafe { LAST_ERR = None };
    0
}

/// `dlerror()`：取最近一次错误描述（无错误返回 NULL）。
///
/// 返回的指针指向 rtld 内部存储，**下次调用可能被覆盖**（POSIX 同约定）。
#[unsafe(no_mangle)]
pub extern "C" fn dlerror() -> *const u8 {
    let e = unsafe { &*core::ptr::addr_of!(LAST_ERR) };
    match e {
        Some(s) => {
            // **不能直接返回 s.as_ptr()**：String / Vec<u8> 不保证末尾有 NUL，
            // C 的 %s 会越过缓冲区末尾继续读（实测粘上无关路径片段）。
            // 显式补 NUL，把「最近的错误」变成**合法的 C 串**。
            let v = unsafe { &mut *core::ptr::addr_of_mut!(LAST_ERR_C) };
            v.clear();
            v.extend_from_slice(s.as_bytes());
            v.push(0);
            v.as_ptr()
        }
        None => core::ptr::null(),
    }
}

/// rtld 自己提供给被装载程序的符号（`resolve` 的**内置表**）。
fn builtin_symbol(name: &str) -> Option<u64> {
    let table: [(&str, usize); 4] = [
        // 先转成函数**指针**再转整数：直接 cast 函数项会被 lint 拒绝（实测警告）。
        ("dlopen", dlopen as *const () as usize),
        ("dlsym", dlsym as *const () as usize),
        ("dlclose", dlclose as *const () as usize),
        ("dlerror", dlerror as *const () as usize),
    ];
    table.iter().find(|(n, _)| *n == name).map(|(_, a)| *a as u64)
}

#[boruix_std::main]
fn main() {
    let argv: Vec<&[u8]> = args().collect();
    println!("[rtld] BORUIX 用户态动态链接器（阶段 5 / 3P5-1）");
    let Some(raw_path) = argv.first() else {
        println!("[rtld] 用法: rtld <program> [symbol]");
        return;
    };
    let path = match core::str::from_utf8(raw_path) {
        Ok(s) => s,
        Err(_) => {
            println!("[rtld] 路径不是合法 UTF-8");
            return;
        }
    };
    // 装载结果放进**全局表**：`dlopen` 运行期追加的对象要与启动期装载的处于同一作用域。
    match load_closure(path) {
        Ok(mut o) => {
            // 启动期对象同样分配**稳定句柄**、引用计数记为 -1（永不卸载）：
            // 这样 RTLD_NOLOAD 对主程序与其依赖也能如实返回同一句柄，
            // 而不是与「未装载」无法区分。
            for ob in o.iter_mut() {
                ob.handle = next_handle();
                ob.refs = -1;
            }
            unsafe { OBJECTS = o }
        }
        Err(e) => {
            println!("[rtld] 装载失败: {}", e);
            return;
        }
    }
    let objs = unsafe { &*core::ptr::addr_of!(OBJECTS) };
    println!("[rtld] 共装载 {} 个对象", objs.len());
    for (i, o) in objs.iter().enumerate() {
        println!("[rtld]   [{}] {} base=0x{:x}", i, o.path, o.base);
    }
    match relocate_all(&objs) {
        Ok(n) => println!("[rtld] 已应用 {} 个重定位", n),
        Err(e) => {
            println!("[rtld] 重定位失败: {}", e);
            return;
        }
    }
    // 验收路径：调用主对象导出的 check()。
    let Some(addr) = resolve(&objs, "check") else {
        println!("[rtld] 未找到符号 check");
        return;
    };
    println!("[rtld] 调用 check @ 0x{:x}", addr);
    let f: extern "C" fn() -> i32 = unsafe { core::mem::transmute(addr) };
    let r = f();
    println!("[rtld] check() 返回 {}", r);
    println!(
        "[rtld] 判定: {}",
        if r == 1 {
            "通过（跨模块调用打通）"
        } else {
            "失败"
        }
    );
}