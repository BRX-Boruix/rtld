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
 * 清单第 6 项验收（三）新增（地址空间真回收）：
 *   ⑥ 卸载前 add 所在页必须**已映射**（给"归还"提供对照，否则 ⑦ 可能只是本来就没映射）；
 *   ⑦ 最后一个引用释放后，**同一页必须未映射**（memory_query 返回 0）——旧实现
 *      dlclose 不 munmap，这一条必红；
 *   ⑧ **依赖者保护**：libwrap.so 对 libadd.so 有真实 DT_NEEDED。在世期间 dlclose(libadd)
 *      必须被**如实拒绝**（返回 -1 且 dlerror 指明是谁依赖）；卸载 libwrap 后才允许，
 *      且那时 libadd 的页才真正变为未映射。
 */
extern void *dlopen(const char *path, int flags);
extern void *dlsym(void *handle, const char *name);
extern int dlclose(void *handle);
extern char *dlerror(void);
extern int printf(const char *fmt, ...);
/* 内核 SYS_MEMORY_QUERY(0x22)：把 addr 所在 4KiB 页的页表位图写入 *out，
 * *out == 0 表示该页**未映射**。
 *
 * 为什么在夹具里内联系统调用而不是调 libc：libc **没有**这个出口——内核
 * (kernel/src/syscall.rs 的 sys_memory_query) 与 libsys (nr.rs 的 SYS_MEMORY_QUERY)
 * 两侧都有，缺的只是 C ABI 包装。而本夹具必须只用盘上 .so 的**现有**导出面。
 * 寄存器约定与 csrc/crtrt.c 的 boruix_syscall 完全一致：rax=nr, rdi/rsi/rdx=a1..a3。 */
static int mem_query(unsigned long addr, unsigned long *out) {
    long ret;
    __asm__ volatile("int $0x80"
                     : "=a"(ret)
                     : "a"(0x22L), "D"(addr), "S"((unsigned long)out)
                     : "rcx", "r11", "memory");
    return (ret < 0) ? -1 : 0;
}

#define RTLD_NOW    2
#define RTLD_NOLOAD 4

#define LIBADD  "/volumes/BORUIX_DATA/3p/libadd.so"
#define LIBWRAP "/volumes/BORUIX_DATA/3p/libwrap.so"

static unsigned long page_of(const void *p) {
    return ((unsigned long)p) & ~0xFFFUL;
}

static int has_sub(const char *s, const char *sub) {
    int i, j;
    if (!s) return 0;
    for (i = 0; s[i]; i++) {
        for (j = 0; sub[j] && s[i + j] == sub[j]; j++) {}
        if (!sub[j]) return 1;
    }
    return 0;
}

int check(void) {
    int f1 = 0, f2 = 0, f3 = 0, f4 = 0, f5 = 0, f6 = 0, f7 = 0, f8 = 0;

    void *h1 = dlopen(LIBADD, RTLD_NOW);
    if (h1 == 0) {
        printf("dlclose: FATAL dlopen 失败: %s\n", dlerror());
        return 0;
    }
    void *h2 = dlopen(LIBADD, RTLD_NOW);
    int same = (h2 == h1);

    int (*add)(int, int) = (int (*)(int, int)) dlsym(h1, "add");
    int r = add ? add(20, 22) : -1;

    /* ⑥ 卸载前的对照：add 所在页必须是已映射的。 */
    {
        unsigned long before = 0;
        if (add && mem_query(page_of((void *)add), &before) == 0 && before != 0)
            f6 = 1;
    }

    /* ② 两次引用各 dlclose 一次；① 返回值必须是 0。 */
    int c1 = dlclose(h2);
    {
        const char *e = dlerror();
        printf("dlclose: diag c1=%d err=%s\n", c1, e ? e : "(none)");
    }
    int c2 = dlclose(h1);
    {
        const char *e = dlerror();
        printf("dlclose: diag c2=%d err=%s\n", c2, e ? e : "(none)");
    }
    if (c1 == 0 && c2 == 0) f1 = 1;
    if (same && r == 42) f2 = 1;

    /* ⑦ 地址空间真归还：同一页现在必须未映射（旧实现不 munmap ⇒ 此处必红）。 */
    {
        unsigned long after = 0xFFFF;
        if (add && mem_query(page_of((void *)add), &after) == 0 && after == 0)
            f7 = 1;
    }

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

    /* ⑧ 依赖者保护 + 卸载顺序。 */
    {
        void *ha = dlopen(LIBADD, RTLD_NOW);
        void *hw = dlopen(LIBWRAP, RTLD_NOW);
        int refused = 0, order_ok = 0, freed = 0;
        if (ha && hw) {
            int (*wrap)(int, int) = (int (*)(int, int)) dlsym(hw, "wrap_add");
            int (*add2)(int, int) = (int (*)(int, int)) dlsym(ha, "add");
            int wr = wrap ? wrap(1, 2) : -1;
            int d1 = dlclose(ha); /* 必须 -1：libwrap 仍依赖它 */
            const char *e = dlerror();
            if (d1 == -1 && has_sub(e, "libwrap.so")) refused = 1;
            int d2 = dlclose(hw); /* 先卸依赖者 */
            int d3 = dlclose(ha); /* 现在才允许 */
            if (d2 == 0 && d3 == 0 && wr == 3) order_ok = 1;
            if (add2) {
                unsigned long q = 0xFFFF;
                if (mem_query(page_of((void *)add2), &q) == 0 && q == 0) freed = 1;
            }
        }
        if (refused && order_ok && freed) f8 = 1;
        printf("dlclose: (8)dep-refused=%d order-ok=%d freed=%d\n", refused, order_ok, freed);
    }

    printf("dlclose: (1)ret0=%d (2)same-handle+add42=%d (3)gone-after-last=%d"
           " (4)null->-1=%d (5)dlerror-nul-terminated=%d (6)page-mapped-before=%d"
           " (7)page-unmapped-after=%d (8)dep-guard=%d  add(20,22)=%d\n",
           f1, f2, f3, f4, f5, f6, f7, f8, r);
    int ok = f1 && f2 && f3 && f4 && f5 && f6 && f7 && f8;
    printf("dlclose: %s (fails=%d)\n", ok ? "PASS" : "FAIL", 8 - (f1 + f2 + f3 + f4 + f5 + f6 + f7 + f8));
    return ok;
}
