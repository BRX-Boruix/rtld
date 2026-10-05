/* 3P5-3 边界验收：dlopen 一个**带 PT_TLS** 的 .so 必须失败（ADR-053 D4）。
 *
 * 为什么必须拒绝：initial-exec 的 TLS 偏移要在**进程启动时**确定；运行期追加会改变已有
 * 模块的偏移，已装载模块的 GOT 槽会全部失效。故本实现如实拒绝，而不是悄悄装上一个坏库。
 */
extern void *dlopen(const char *path, int flags);
extern char *dlerror(void);
extern int printf(const char *fmt, ...);

#define RTLD_NOW 2

int check(void) {
    void *h = dlopen("/volumes/BORUIX_DATA/3p/libtls.so", RTLD_NOW);
    if (h != 0) {
        printf("C: 竟然装上了带 TLS 的 .so——违反 ADR-053 D4\n");
        return 0;
    }
    printf("C: 已如实拒绝: %s\n", dlerror());
    return 1;
}