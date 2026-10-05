//! rtld —— BORUIX 用户态动态链接器（阶段 5 / 3P5-1）。
//!
//! ## 为什么是"独立程序"而不是内核自动装载（架构决策）
//!
//! 本系统的 `exec` 是**用户态读好镜像再交给内核**的（`Image::Slice`/`Image::Shm`），
//! 内核**没有**"按路径读程序"的能力；而标准 `PT_INTERP` 自动装载要求内核按路径读解释器。
//! 故采用 **`ld.so` 模型**：`rtld <program> [args...]`，rtld 自己读文件、自己 mmap、
//! 自己做重定位。**内核不需要理解 ELF 重定位**（S13 边界）。
//!
//! ## 当前能力（本文件）
//!
//! 1. 解析 ELF64 头与程序头；
//! 2. 把全部 `PT_LOAD` 按 `p_vaddr` 的相对布局**一次性 mmap**（保证段间相对偏移不变），
//!    拷贝文件内容、补零 bss、按 `p_flags` 逐段 `mprotect`；
//! 3. 解析 `PT_DYNAMIC`，应用 **`R_X86_64_RELATIVE`** 重定位；
//! 4. 经 `DT_SYMTAB`/`DT_STRTAB`/`DT_HASH` 查导出符号；
//! 5. 调用查到的函数（用于验收；将来改为跳到 `e_entry`）。
//!
//! **尚未做（如实声明）**：`DT_NEEDED` 递归装载、`GLOB_DAT`/`JUMP_SLOT` 重定位、
//! `.so` 级 TLS（ADR-053）。遇到这些重定位类型时**如实报错**，绝不静默跳过。

#![allow(dead_code)]

extern crate alloc;

use alloc::vec::Vec;
use boruix_std::{args, print, println};

const PAGE: u64 = 4096;

// ---------- ELF64 结构 ----------

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

fn parse_header(b: &[u8]) -> Result<ElfHeader, &'static str> {
    if b.len() < 64 {
        return Err("文件小于 ELF64 头");
    }
    if &b[0..4] != b"\x7fELF" {
        return Err("不是 ELF（魔数不符）");
    }
    if b[4] != 2 {
        return Err("不是 ELF64");
    }
    if b[5] != 1 {
        return Err("不是小端");
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

fn parse_phdrs(b: &[u8], h: &ElfHeader) -> Result<Vec<Phdr>, &'static str> {
    if h.e_phentsize as usize != 56 {
        return Err("phentsize != 56（非 ELF64 程序头）");
    }
    let mut v = Vec::new();
    for i in 0..h.e_phnum as u64 {
        let off = (h.e_phoff + i * 56) as usize;
        if off + 56 > b.len() {
            return Err("程序头表越界");
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

// ---------- 装载 ----------

/// 装载结果：段区间的基址 + `PT_DYNAMIC` 的用户 VA。
struct Loaded {
    base: u64,
    dynamic: Option<u64>,
}

fn align_down(v: u64) -> u64 {
    v & !(PAGE - 1)
}

fn align_up(v: u64) -> u64 {
    (v + PAGE - 1) & !(PAGE - 1)
}

fn prot_of(flags: u32) -> u64 {
    // p_flags: 1=X 2=W 4=R
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

/// 装载一个 ET_DYN 对象：一次 mmap 覆盖全部 PT_LOAD 的跨度（保证段间相对偏移不变）。
fn load_object(image: &[u8], phdrs: &[Phdr]) -> Result<Loaded, &'static str> {
    let loads: Vec<&Phdr> = phdrs.iter().filter(|p| p.p_type == 1).collect();
    if loads.is_empty() {
        return Err("没有 PT_LOAD");
    }
    let min_va = loads.iter().map(|p| align_down(p.p_vaddr)).min().unwrap();
    let max_va = loads.iter().map(|p| align_up(p.p_vaddr + p.p_memsz)).max().unwrap();
    let span = max_va - min_va;
    if span == 0 {
        return Err("段跨度为 0");
    }
    // 先按 RW 映射整段（拷贝内容需要写权限），随后逐段收紧。
    let base = match boruix_std::libsys::mmap_prot(span, 1 | 2) {
        Ok(a) => a,
        Err(e) => {
            println!("[rtld] mmap({}) 失败: {:?}", span, e);
            return Err("mmap 失败");
        }
    };
    // 拷贝每个段的内容 + 补零 bss。
    for p in &loads {
        let dst = base + (p.p_vaddr - min_va);
        let src_off = p.p_offset as usize;
        let filesz = p.p_filesz as usize;
        if src_off + filesz > image.len() {
            return Err("段内容越界");
        }
        unsafe {
            core::ptr::copy_nonoverlapping(image.as_ptr().add(src_off), dst as *mut u8, filesz);
            // bss：filesz..memsz 清零。
            let bss = (p.p_memsz - p.p_filesz) as usize;
            if bss > 0 {
                core::ptr::write_bytes((dst + p.p_filesz) as *mut u8, 0, bss);
            }
        }
    }
    // 逐段收紧权限（W^X 由内核单点裁决；这里只按 p_flags 如实申请）。
    for p in &loads {
        let start = align_down(base + (p.p_vaddr - min_va));
        let end = align_up(base + (p.p_vaddr - min_va) + p.p_memsz);
        if let Err(e) = boruix_std::libsys::mprotect(start, end - start, prot_of(p.p_flags)) {
            println!("[rtld] mprotect(0x{:x}, {}) 失败: {:?}", start, end - start, e);
            return Err("mprotect 失败");
        }
    }
    let dynamic = phdrs
        .iter()
        .find(|p| p.p_type == 2)
        .map(|p| base + (p.p_vaddr - min_va));
    Ok(Loaded { base, dynamic })
}

// ---------- PT_DYNAMIC ----------
//
// 本表是 ELF 动态节的 **ABI 常量**：按名字列全，便于对照与下一步使用。
// 当前只消费其中一部分，故在 crate 根显式放行未用项（而不是删掉——删掉会让下一个人重新查手册）。

const DT_NULL: u64 = 0;
const DT_HASH: u64 = 4;
const DT_STRTAB: u64 = 5;
const DT_SYMTAB: u64 = 6;
const DT_RELA: u64 = 7;
const DT_RELASZ: u64 = 8;
const DT_RELAENT: u64 = 9;
const DT_STRSZ: u64 = 10;
const DT_SYMENT: u64 = 11;
const DT_NEEDED: u64 = 1;
const DT_JMPREL: u64 = 23;
const DT_PLTRELSZ: u64 = 2;

const R_X86_64_GLOB_DAT: u32 = 6;
const R_X86_64_JUMP_SLOT: u32 = 7;
const R_X86_64_RELATIVE: u32 = 8;

/// 该 DT tag 的值是否为**虚拟地址**（装载后须加基址才能解引用）。
///
/// 非地址型（`SIZE`/`ENT`/`NEEDED`/`COUNT`/`SONAME` 等）的值是标量或字符串表偏移，
/// **不得**加基址。此分类是 ELF 规范事实，集中在此单点（S15）。
fn is_dynamic_addr(tag: u64) -> bool {
    matches!(tag, 3 | 4 | 5 | 6 | 7 | 12 | 13 | 17 | 23 | 25 | 26 | 0x6ffffef5)
}

/// 解析 PT_DYNAMIC，返回 `(tag, val)` 列表。
///
/// **地址型 tag 的值在这里统一加上 `base`**（S15 单点）：ELF 动态节里的
/// `DT_SYMTAB`/`DT_STRTAB`/`DT_HASH`/`DT_RELA`/`DT_JMPREL` 等是**虚拟地址**，装载到
/// 任意基址后必须加偏移。此前直接用原值解引用 → 访问低地址 → SIGSEGV（实测退出码 11）。
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

/// 应用重定位。只支持 `R_X86_64_RELATIVE`；其余**如实报错**（S09）。
fn apply_relocations(d: &[(u64, u64)], base: u64) -> Result<usize, &'static str> {
    let Some(rela) = dyn_get(d, DT_RELA) else {
        return Ok(0);
    };
    let relasz = dyn_get(d, DT_RELASZ).unwrap_or(0);
    let relaent = dyn_get(d, DT_RELAENT).unwrap_or(24);
    if relaent != 24 {
        return Err("RELAENT != 24");
    }
    let mut n = 0usize;
    let mut off = 0u64;
    while off + 24 <= relasz {
        let e = rela + off;
        let r_offset = unsafe { *(e as *const u64) };
        let r_info = unsafe { *((e + 8) as *const u64) };
        let r_addend = unsafe { *((e + 16) as *const u64) };
        let r_type = (r_info & 0xffff_ffff) as u32;
        match r_type {
            R_X86_64_RELATIVE => {
                let v = base.wrapping_add(r_addend);
                unsafe { *((base + r_offset) as *mut u64) = v };
                n += 1;
            }
            other => {
                println!("[rtld] 不支持的重定位类型 {} @ 0x{:x}", other, r_offset);
                return Err("遇到不支持的重定位类型（未静默跳过）");
            }
        }
        off += 24;
    }
    // .rela.plt（JUMP_SLOT）若存在但未实现，必须**明确报出**而不是当作没有。
    if dyn_get(d, DT_JMPREL).is_some() && dyn_get(d, DT_PLTRELSZ).unwrap_or(0) > 0 {
        return Err("存在 .rela.plt（JUMP_SLOT），尚未实现");
    }
    Ok(n)
}

/// 经 DT_HASH 的 nchain 得到符号数，再按名字线性查找（本阶段够用；将来可换 GNU_HASH）。
fn lookup_symbol(d: &[(u64, u64)], name: &str) -> Option<u64> {
    let symtab = dyn_get(d, DT_SYMTAB)?;
    let strtab = dyn_get(d, DT_STRTAB)?;
    let hash = dyn_get(d, DT_HASH)?;
    let nchain = unsafe { *((hash + 4) as *const u32) } as usize;
    let syment = dyn_get(d, DT_SYMENT).unwrap_or(24);
    if syment != 24 {
        return None;
    }
    for i in 0..nchain {
        let s = symtab + (i as u64) * 24;
        let st_name = unsafe { *(s as *const u32) } as u64;
        let st_value = unsafe { *((s + 8) as *const u64) };
        if st_name == 0 {
            continue;
        }
        let p = strtab + st_name;
        let mut ok = true;
        for (k, c) in name.bytes().enumerate() {
            let got = unsafe { *((p + k as u64) as *const u8) };
            if got != c {
                ok = false;
                break;
            }
        }
        if ok && unsafe { *((p + name.len() as u64) as *const u8) } == 0 {
            return Some(st_value);
        }
    }
    None
}

fn show_arg(a: &[u8]) {
    for &c in a {
        if (0x20..0x7f).contains(&c) {
            print!("{}", c as char);
        } else {
            print!("\\x{:02x}", c);
        }
    }
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
    let image = match boruix_std::fs::read_to_end(path) {
        Ok(v) => v,
        Err(e) => {
            println!("[rtld] 读取 {:?} 失败: {:?}", path, e);
            return;
        }
    };
    println!("[rtld] 目标 {} 字节", image.len());
    let h = match parse_header(&image) {
        Ok(h) => h,
        Err(e) => {
            println!("[rtld] 解析失败: {}", e);
            return;
        }
    };
    println!(
        "[rtld] ELF: type={} machine={} entry=0x{:x} phnum={}",
        h.e_type, h.e_machine, h.e_entry, h.e_phnum
    );
    let phdrs = match parse_phdrs(&image, &h) {
        Ok(p) => p,
        Err(e) => {
            println!("[rtld] 程序头解析失败: {}", e);
            return;
        }
    };
    let loaded = match load_object(&image, &phdrs) {
        Ok(l) => l,
        Err(e) => {
            println!("[rtld] 装载失败: {}", e);
            return;
        }
    };
    println!("[rtld] 已装载 base=0x{:x} dynamic={:?}", loaded.base, loaded.dynamic);
    let Some(dyn_va) = loaded.dynamic else {
        println!("[rtld] 该镜像没有 PT_DYNAMIC（静态程序无需 rtld）");
        return;
    };
    let d = read_dynamic(dyn_va, loaded.base);
    println!("[rtld] DT_* 条目 {} 个", d.len());
    match apply_relocations(&d, loaded.base) {
        Ok(n) => println!("[rtld] 已应用 {} 个 R_X86_64_RELATIVE", n),
        Err(e) => {
            println!("[rtld] 重定位失败: {}", e);
            return;
        }
    }
    // 验收路径：调用目标程序导出的 check()，由它验证重定位是否生效。
    match lookup_symbol(&d, "check") {
        Some(off) => {
            let addr = loaded.base + off;
            println!("[rtld] 找到 check @ 0x{:x}，调用它", addr);
            let f: extern "C" fn() -> i32 = unsafe { core::mem::transmute(addr) };
            let r = f();
            println!("[rtld] check() 返回 {}", r);
            println!("[rtld] 判定: {}", if r == 1 { "通过（重定位生效）" } else { "失败" });
        }
        None => println!("[rtld] 未找到符号 check（该镜像可能不是本轮的验收对象）"),
    }
    let _ = argv.get(1);
}