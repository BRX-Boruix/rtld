/* dlfcn.h —— 运行期装载（阶段 5 / 3P5-3）。
 *
 * **这些符号由 rtld 提供，不是 libc**：`dlopen` 需要 rtld 自己的装载器状态（已装载对象表）。
 * 被装载程序里的 `dlopen` 是 UND，由 rtld 的符号解析**内置表**命中。
 *
 * 因此本头文件**不在 libc 的 sysroot 里**——用它的程序必须由 rtld 运行（`rtld prog`）。
 */
#ifndef _DLFCN_H
#define _DLFCN_H

/* POSIX 取值（本实现一律立即解析全部重定位，故 LAZY 与 NOW 行为相同，如实声明）。 */
#define RTLD_LAZY   1
#define RTLD_NOW    2
#define RTLD_GLOBAL 0x100
#define RTLD_LOCAL  0

void *dlopen(const char *path, int flags);
void *dlsym(void *handle, const char *name);
char *dlerror(void);

#endif /* _DLFCN_H */