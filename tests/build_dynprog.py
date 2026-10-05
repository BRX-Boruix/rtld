#!/usr/bin/env python3
"""构建"共享 libc"的验收夹具（阶段 5 / 3P5-2）。

用法: python build_dynprog.py [源文件名...]
      默认构建 dynprog.c 与 dynprog2.c，产出同名 .elf。

依赖 `libboruixc.so.1`（由 tools/tools_build/build_so.py 产出）。
用 `-l:libboruixc.so.1` 精确指定文件名——该 `.so` 只有带版本号的真名，没有 `libX.so` 链接名。

工具链路径不写死（S01）：经 BORUIX_CLANG / BORUIX_LLD 或 PATH 取得。
"""
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
CLANG = os.environ.get("BORUIX_CLANG") or "clang"
LLD = os.environ.get("BORUIX_LLD") or "ld.lld"
TARGET = "--target=x86_64-unknown-none"
SO = os.path.join(ROOT, "libc", "target", "x86_64-unknown-boruix", "release", "libboruixc.so.1")


def run(cmd):
    print("+", " ".join(cmd))
    r = subprocess.run(cmd)
    if r.returncode != 0:
        sys.exit("命令失败（%d）" % r.returncode)


def build(stem):
    src = os.path.join(HERE, stem + ".c")
    obj = os.path.join(HERE, stem + ".o")
    out = os.path.join(HERE, stem + ".elf")
    run([CLANG, TARGET, "-ffreestanding", "-fno-builtin", "-fPIE", "-O2", "-c", src, "-o", obj])
    # `--unresolved-symbols=ignore-in-object-files`：`dlopen`/`dlsym`/`dlerror` 由 **rtld 运行期
    # 提供**（不在任何 .so 里），链接期必然未解析。这个旗标把它们**保留为动态未定义符号**，
    # 交给 rtld 在运行期解析（解析不到时 rtld 如实报错）。
    # **不要用 `-u`**：实测对「源码里没引用的名字」用 `-u` 会让 lld 报 undefined（`dlprog_tls`
    # 只用 dlopen/dlerror，却因 `-u dlsym` 而链接失败）。**更不要用
    # `--unresolved-symbols=ignore-all`**：它会把符号从动态符号表里删掉，PLT 解析到 0。
    run([LLD, "-pie", "-e", "check", "--no-dynamic-linker", "-z", "norelro",
         "--unresolved-symbols=ignore-in-object-files",
         "--export-dynamic", "-o", out, obj,
         "-L" + os.path.dirname(SO), "-l:libboruixc.so.1"])
    print("[build_dynprog] 产出", out)


def main():
    if not os.path.isfile(SO):
        sys.exit("找不到 %s——先跑 tools/tools_build/build_so.py" % SO)
    stems = sys.argv[1:] or ["dynprog", "dynprog2"]
    for s in stems:
        build(os.path.splitext(s)[0])


if __name__ == "__main__":
    main()