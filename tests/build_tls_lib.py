#!/usr/bin/env python3
"""构建 D4 边界验收用的**带 TLS** 共享库（阶段 5 / 3P5-3）。

产物：libtls.so —— 一个带 `PT_TLS` 的 `.so`。
`dlopen` 它必须**失败**（ADR-053 D4）：initial-exec 的 TLS 偏移必须在**进程启动时**确定，
运行期追加会改变已有模块的偏移，已装载模块的 GOT 槽会全部失效。

用 `-ftls-model=initial-exec` 是刻意的：这正是 ADR-053 D1 为 `.so` 选定的模型。

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
    obj = os.path.join(HERE, "tls_lib.o")
    out = os.path.join(HERE, "libtls.so")
    run([CLANG, TARGET, "-ffreestanding", "-fno-builtin", "-fPIC", "-O2",
         "-ftls-model=initial-exec", "-c", os.path.join(HERE, "tls_lib.c"), "-o", obj])
    run([LLD, "-shared", "-o", out, obj])
    print("[build_tls_lib] 产出", out)


if __name__ == "__main__":
    main()