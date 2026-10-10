/* 清单第 6 项验收（一）：dlopen 的 RTLD_NOLOAD 旗标。
 *
 * dlopen/dlsym/dlerror 由 **rtld** 提供（不是 libc）：它们在链接期是 UND，由 rtld 的
 * 符号解析**内置表**命中。所以本程序必须由 rtld 运行（rtld <prog>.elf）。
 *
 * 三条**可判定的**契约（修复前每一条都必红）：
 *   ① 未装载时 dlopen(path, RTLD_NOLOAD) 必须返回 NULL（**绝不装载**）；
 *   ② 已装载后同路径第二次 dlopen(RTLD_NOW) 必须返回**同一句柄**（引用计数去重），
 *      而不是同一 .so 的第二份副本；
 *   ③ 已装载后 dlopen(path, RTLD_NOLOAD) 必须返回**同一句柄**。
 *
 * 报告一次性打印（串口多写者会把分行的输出拆碎——方法学记录见 docs/TODO/3p.md）。
 */
extern void *dlopen(const char *path, int flags);
extern void *dlsym(void *handle, const char *name);
extern char *dlerror(void);
extern int printf(const char *fmt, ...);

#define RTLD_NOW    2
#define RTLD_NOLOAD 4

#define LIBADD "/volumes/BORUIX_DATA/3p/libadd.so"

int check(void) {
    int f1 = 0, f2 = 0, f3 = 0;
    void *h;

    /* ① 未装载 + RTLD_NOLOAD ⇒ NULL，且 dlerror 必须给出原因。 */
    h = dlopen(LIBADD, RTLD_NOLOAD);
    if (h == 0 && dlerror() != 0) f1 = 1;

    /* ② 装载，然后同路径再装载一次：必须同一句柄。 */
    void *h1 = dlopen(LIBADD, RTLD_NOW);
    if (h1 != 0) {
        void *h2 = dlopen(LIBADD, RTLD_NOW);
        if (h2 == h1) f2 = 1;

        /* ③ 已装载 + RTLD_NOLOAD ⇒ 同一句柄。 */
        void *h3 = dlopen(LIBADD, RTLD_NOLOAD);
        if (h3 == h1) f3 = 1;
    }

    printf("noload: (1)unloaded->NULL=%d (2)reopen-same-handle=%d (3)noload-same-handle=%d"
           "  h1=%p  err=%s\n",
           f1, f2, f3, h1, dlerror() ? dlerror() : "(none)");
    int ok = f1 && f2 && f3;
    printf("noload: %s (fails=%d)\n", ok ? "PASS" : "FAIL", 3 - (f1 + f2 + f3));
    return ok;
}
