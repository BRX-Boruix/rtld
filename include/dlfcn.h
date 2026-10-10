/* dlfcn.h —— 运行期装载（阶段 5 / 3P5-3）。
 *
 * **这些符号由 rtld 提供，不是 libc**：`dlopen` 需要 rtld 自己的装载器状态（已装载对象表）。
 * 被装载程序里的 `dlopen` 是 UND，由 rtld 的符号解析**内置表**命中。
 *
 * 因此本头文件**不在 libc 的 sysroot 里**——用它的程序必须由 rtld 运行（`rtld prog`）。
 */
#ifndef _DLFCN_H
#define _DLFCN_H

/* 取值与 glibc 一致（编译期常量，无 ABI 含义）。**实际支持面**见 rtld/src/main.rs 的
 * dlopen/dlclose 文档：LAZY 与 NOW 行为相同（一律立即解析全部重定位）；GLOBAL/LOCAL
 * 语义支持；NOLOAD 命中即返回同一句柄、未命中返回 NULL 且**不装载**；
 * NODELETE 置位时 dlclose 不摘除对象。未知旗标位**如实拒绝**（不静默忽略）。 */
#define RTLD_LAZY     0x00001
#define RTLD_NOW      0x00002
#define RTLD_NOLOAD   0x00004
#define RTLD_GLOBAL   0x00100
#define RTLD_LOCAL    0x00000
#define RTLD_NODELETE 0x01000

void *dlopen(const char *path, int flags);
void *dlsym(void *handle, const char *name);
/* 成功 0；句柄非法/未知 -1（同时置 dlerror）。 */
int   dlclose(void *handle);
char *dlerror(void);

#endif /* _DLFCN_H */