/* 清单第 6 项验收（二）：dlclose / RTLD_NOLOAD 句柄语义 + dlerror 的 C 串契约。
 *
 * dlclose/dlerror 由 **rtld** 提供（不是 libc），故本程序必须由 rtld 运行。
 *
 * 契约（修复前每一条都必红）：
 *   ① dlclose(handle) 存在且可调用，成功返回 0；
 *   ② 引用计数语义：同路径 dlopen 两次得到**同一句柄**，两次 dlclose 各自递减一次；
 *   ③ 最后一个引用释放后，dlopen(path, RTLD_NOLOAD) 必须返回 NULL（对象已摘除）；
 *   ④ dlclose(NULL) 如实返回 -1（句柄非法），不静默成功；
 *   ⑤ **dlerror() 必须返回 NUL 结尾的合法 C 串**——不是"看起来像"，而是
 *      第 strlen 个字节**就是 0**。这条是**实测踩出来的**：修复前机内原始输出是
 *      `err=dlclose: 句柄非法3p/libadd.s`——错误串后面粘着无关的路径片段，
 *      因为实现直接把 Rust String 的缓冲区指针交给了 C 的 %s（String 不保证末尾有 NUL）。
 *
 * **如实声明**：本实现 dlclose 不 munmap（地址空间不回收），只把对象移出符号解析作用域。
 * 见 rtld/src/main.rs 的 dlclose 文档。
 */
extern void *dlopen(const char *path, int flags);
extern void *dlsym(void *handle, const char *name);
extern int dlclose(void *handle);
extern char *dlerror(void);
extern int printf(const char *fmt, ...);

#define RTLD_NOW    2
#define RTLD_NOLOAD 4

#define LIBADD "/volumes/BORUIX_DATA/3p/libadd.so"

int check(void) {
    int f1 = 0, f2 = 0, f3 = 0, f4 = 0, f5 = 0;

    void *h1 = dlopen(LIBADD, RTLD_NOW);
    if (h1 == 0) {
        printf("dlclose: FATAL dlopen 失败: %s\n", dlerror());
        return 0;
    }
    void *h2 = dlopen(LIBADD, RTLD_NOW);
    int same = (h2 == h1);

    int (*add)(int, int) = (int (*)(int, int)) dlsym(h1, "add");
    int r = add ? add(20, 22) : -1;

    /* ② 两次引用各 dlclose 一次；① 返回值必须是 0。 */
    int c1 = dlclose(h2);
    int c2 = dlclose(h1);
    if (c1 == 0 && c2 == 0) f1 = 1;
    if (same && r == 42) f2 = 1;

    /* ③ 引用归零 ⇒ 已摘除。 */
    void *h3 = dlopen(LIBADD, RTLD_NOLOAD);
    if (h3 == 0) f3 = 1;

    /* ④ 非法句柄如实报错；⑤ 紧随其后的 dlerror() 必须是 NUL 结尾的 C 串。 */
    if (dlclose(0) == -1) f4 = 1;
    {
        /* 期望文本是**本实现文档化的错误串**；此处比较到结尾的 NUL 为止，
           目的不是"文案一致"，而是**证明结尾确实有一个 NUL**。 */
        static const char expect[] = "dlclose: 句柄非法";
        int n = 0, k;
        const char *e;
        while (expect[n]) n++;
        e = dlerror();
        if (e) {
            for (k = 0; k <= n; k++)
                if (e[k] != expect[k]) break;
            if (k == n + 1) f5 = 1;
        }
    }

    printf("dlclose: (1)ret0=%d (2)same-handle+add42=%d (3)gone-after-last=%d"
           " (4)null->-1=%d (5)dlerror-nul-terminated=%d  add(20,22)=%d\n",
           f1, f2, f3, f4, f5, r);
    int ok = f1 && f2 && f3 && f4 && f5;
    printf("dlclose: %s (fails=%d)\n", ok ? "PASS" : "FAIL", 5 - (f1 + f2 + f3 + f4 + f5));
    return ok;
}
