//! rtld —— BORUIX 用户态动态链接器（阶段 5 / 3P5-1）。
//!
//! ## 为什么是"独立程序"而不是内核自动装载（架构决策）
//!
//! 本系统的 `exec` 是**用户态读好镜像再交给内核**的（`Image::Slice`/`Image::Shm`），
//! 内核**没有**"按路径读程序"的能力；而标准的 `PT_INTERP` 自动装载要求内核按
//! `PT_INTERP` 里的路径去读解释器。故本阶段采用 **`ld.so` 模型**：
//!
//! ```text
//!   rtld /path/to/program [args...]
//! ```
//!
//! rtld 自己读程序文件、自己 `mmap` 各段、自己做重定位、最后跳到程序入口。
//! **内核不需要理解 ELF 重定位**（S13 边界）。将来若要支持"按名直接运行动态程序"
//! （`PT_INTERP` 自动装载），内核侧前置已就绪（loader 已能解析并记录解释器路径、
//! auxv 五项已补齐）。
//!
//! ## 本文件当前阶段
//!
//! **行走骨架**：读 `argv[0]`（本系统 ABI 经 shell 派生时已剥程序名，故首个词即第一个
//! 参数）、打开并读入该文件、解析 ELF64 头部，把结果打印出来。先证明"能被起起来 +
//! 能读文件 + 能解析 ELF"这条链是活的，再往上加 mmap/重定位/装载。

extern crate alloc;

use alloc::vec::Vec;
use boruix_std::{args, print, println};

/// ELF64 文件头里的关键字段（只取本阶段要用的）。
#[derive(Debug, Clone, Copy)]
struct ElfHeader {
    e_type: u16,
    e_machine: u16,
    e_entry: u64,
    e_phoff: u64,
    e_phentsize: u16,
    e_phnum: u16,
}

fn rd_u16(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}

fn rd_u64(b: &[u8], off: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[off..off + 8]);
    u64::from_le_bytes(a)
}

/// 解析 ELF64 头。**如实报错**：魔数/位数/字节序不对一律拒绝（S09）。
fn parse_header(b: &[u8]) -> Result<ElfHeader, &'static str> {
    if b.len() < 64 {
        return Err("文件小于 ELF64 头");
    }
    if &b[0..4] != b"\x7fELF" {
        return Err("不是 ELF（魔数不符）");
    }
    if b[4] != 2 {
        return Err("不是 ELF64（EI_CLASS != 2）");
    }
    if b[5] != 1 {
        return Err("不是小端（EI_DATA != 1）");
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

fn type_name(t: u16) -> &'static str {
    match t {
        2 => "ET_EXEC",
        3 => "ET_DYN",
        _ => "其它",
    }
}

/// 按字节打印参数（本系统 ABI 不假设 UTF-8，S02）。
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
    println!("[rtld] BORUIX 用户态动态链接器（行走骨架）");
    println!("[rtld] 参数个数 = {}", argv.len());
    for (i, a) in argv.iter().enumerate() {
        print!("[rtld]   [{}] = ", i);
        show_arg(a);
        println!("");
    }
    let Some(raw_path) = argv.first() else {
        println!("[rtld] 用法: rtld <program> [args...]");
        return;
    };
    // 路径 API 收 &str，故显式做 UTF-8 校验并如实报错（S02：不假设编码，但要给出确定结论）。
    let path = match core::str::from_utf8(raw_path) {
        Ok(s) => s,
        Err(_) => {
            println!("[rtld] 路径不是合法 UTF-8，拒绝");
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
    println!("[rtld] 读到 {} 字节", image.len());
    match parse_header(&image) {
        Ok(h) => println!(
            "[rtld] ELF: type={} machine={} entry=0x{:x} phoff={} phentsize={} phnum={}",
            type_name(h.e_type), h.e_machine, h.e_entry, h.e_phoff, h.e_phentsize, h.e_phnum
        ),
        Err(e) => println!("[rtld] 解析失败: {}", e),
    }
}