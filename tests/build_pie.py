#!/usr/bin/env python3
"""构建 rtld 验收夹具（阶段 5 / 3P5-1）。

产物：prog.elf —— 一个含 R_X86_64_RELATIVE 的 PIE，导出 check 供 rtld 调用。

为什么用 --export-dynamic：PIE 的 .dynsym 默认只含动态链接真正需要的符号，
`check` 不在其中（实测：rtld 按名查找失败）。加该旗标后才进 .dynsym。
"""
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
# 工具链路径不写死（S01）：经环境变量或 PATH 取得。
CLANG = os.environ.get("BORUIX_CLANG") or "clang"
LLD = os.environ.get("BORUIX_LLD") or "ld.lld"


def run(cmd):
    print("+", " ".join(cmd))
    r = subprocess.run(cmd)
    if r.returncode != 0:
        sys.exit("命令失败（%d）" % r.returncode)


def main():
    src = os.path.join(HERE, "pie_reloc.c")
    obj = os.path.join(HERE, "prog.o")
    out = os.path.join(HERE, "prog.elf")
    run([CLANG, "--target=x86_64-unknown-none", "-ffreestanding", "-fno-builtin",
         "-fPIE", "-O2", "-c", src, "-o", obj])
    run([LLD, "-pie", "-e", "check", "--no-dynamic-linker", "-z", "norelro",
         "--export-dynamic", "-o", out, obj])
    print("[build_pie] 产出", out)


if __name__ == "__main__":
    main()