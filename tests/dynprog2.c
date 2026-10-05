/* rtld 验收夹具之四（阶段 5 / 3P5-2）：**第二个**共享 libc 的动态程序。
 *
 * 与 dynprog.c **共享同一份** `libboruixc.so.1`，但用不同的一组 libc 函数：
 * `malloc`（堆）、`snprintf`（含浮点格式化）。
 *
 * 这条用例同时回答两件事：
 *   ① 一份 `.so` 能被**多个**程序共享；
 *   ② 重编 libc、**只替换 `.so`** 之后，**不重链接**的旧程序仍能跑（ABI 冻结的实证）。
 *
 * **注**：这里刻意不用 `sqrt` 等数学函数——它们是 `compiler_builtins` 的**弱符号**，
 * 当前导出面里没有（已记入 docs/TODO/3p.md，属独立待查项），与本用例要验的事无关。
 */
extern void *malloc(unsigned long size);
extern int snprintf(char *buf, unsigned long n, const char *fmt, ...);
extern int printf(const char *fmt, ...);

int check(void) {
    char buf[64];
    void *p = malloc(16);
    if (p == 0) {
        printf("B: malloc 失败\n");
        return 0;
    }
    snprintf(buf, sizeof buf, "%.3f", 1.5);
    printf("B: malloc ok, snprintf=%.3f (buf=%s)\n", 1.5, buf);
    return 1;
}