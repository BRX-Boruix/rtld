#!/usr/bin/env python3
"""构建 rtld 的**动态链接**验收夹具（阶段 5 / 3P5-1）。

产物：
  libadd.so  —— 导出 add 的共享库；
  libwrap.so —— 对 libadd.so 有真实 DT_NEEDED 的库，导出 wrap_add（调用 add）。
                供 dlclose 的「仍有依赖者则拒绝卸载」验收制造场景；
  main.elf   —— 引用 libadd.so 的 PIE（含 DT_NEEDED + JUMP_SLOT/GLOB_DAT 重定位），
                导出 check 供 rtld 调用。

工具链路径不写死（S01）：经 BORUIX_CLANG / BORUIX_LLD 或 PATH 取得。
"""
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
CLANG = os.environ.get("BORUIX_CLANG") or "clang"
LLD = os.environ.get("BORUIX_LLD") or "ld.lld"
TARGET = "--target=x86_64-unknown-none"


def run(cmd):
    print("+", " ".join(cmd))
    r = subprocess.run(cmd)
    if r.returncode != 0:
        sys.exit("命令失败（%d）" % r.returncode)


def main():
    # 共享库：全局符号默认进 .dynsym，无需 --export-dynamic。
    run([CLANG, TARGET, "-ffreestanding", "-fno-builtin", "-fPIC", "-O2",
         "-c", os.path.join(HERE, "libadd.c"), "-o", os.path.join(HERE, "libadd.o")])
    run([LLD, "-shared", "-o", os.path.join(HERE, "libadd.so"),
         os.path.join(HERE, "libadd.o")])
    # 依赖 libadd.so 的第二个库：-ladd 让 lld 在 libwrap.so 里写下 DT_NEEDED libadd.so。
    run([CLANG, TARGET, "-ffreestanding", "-fno-builtin", "-fPIC", "-O2",
         "-c", os.path.join(HERE, "libwrap.c"), "-o", os.path.join(HERE, "libwrap.o")])
    run([LLD, "-shared", "-o", os.path.join(HERE, "libwrap.so"),
         os.path.join(HERE, "libwrap.o"), "-L" + HERE, "-ladd"])
    # 引用方：PIE + --export-dynamic（让 check 进 .dynsym，rtld 才能按名找到它）。
    run([CLANG, TARGET, "-ffreestanding", "-fno-builtin", "-fPIE", "-O2",
         "-c", os.path.join(HERE, "main.c"), "-o", os.path.join(HERE, "main.o")])
    run([LLD, "-pie", "-e", "check", "--no-dynamic-linker", "-z", "norelro",
         "--export-dynamic", "-o", os.path.join(HERE, "main.elf"),
         os.path.join(HERE, "main.o"), "-L" + HERE, "-ladd"])
    print("[build_dyn] 产出 libadd.so + libwrap.so + main.elf")


if __name__ == "__main__":
    main()