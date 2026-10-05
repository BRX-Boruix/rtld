# rtld —— BORUIX 用户态动态链接器

阶段 5（动态链接）的核心组件（`docs/TODO/3p.md` 的 `3P5-1`）。

## 用法

```text
rtld /path/to/program [args...]
```

rtld 自己读程序文件、`mmap` 各段、做重定位，最后跳到程序入口——**内核不需要理解
ELF 重定位**（S13 边界）。

## 为什么是独立程序，而不是内核自动装载 `PT_INTERP`

本系统的 `exec` 是**用户态读好镜像再交给内核**的（`Image::Slice`/`Image::Shm`），内核
**没有**"按路径读程序"的能力；而标准的 `PT_INTERP` 自动装载要求内核按 `PT_INTERP` 里的
路径去读解释器。故本阶段采用 **`ld.so` 模型**。将来若要支持"按名直接运行动态程序"，
内核侧前置已就绪（loader 已能解析并记录解释器路径、auxv 五项已补齐）。

## 构建

```text
set BORUIX_SYSROOT=<install --prefix 的产物>
cargo boruix build
```
