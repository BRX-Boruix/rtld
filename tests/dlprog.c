/* 3P5-3 验收：运行期 dlopen/dlsym。
 *
 * `dlopen`/`dlsym` 由 **rtld** 提供（不是 libc）：它们在链接期是 UND，由 rtld 的符号解析
 * 内置表命中。所以本程序必须由 rtld 运行。
 *
 * 验的是：运行期装载一个此前完全未知的 `.so`、查到符号、调用它。
 */
extern void *dlopen(const char *path, int flags);
extern void *dlsym(void *handle, const char *name);
extern char *dlerror(void);
extern int printf(const char *fmt, ...);

#define RTLD_NOW 2

int check(void) {
    void *h = dlopen("/volumes/BORUIX_DATA/3p/libadd.so", RTLD_NOW);
    if (h == 0) {
        printf("dlopen 失败: %s\n", dlerror());
        return 0;
    }
    int (*add)(int, int) = (int (*)(int, int)) dlsym(h, "add");
    if (add == 0) {
        printf("dlsym 失败: %s\n", dlerror());
        return 0;
    }
    int r = add(20, 22);
    printf("dlopen/dlsym ok: add(20,22)=%d\n", r);
    return r == 42 ? 1 : 0;
}