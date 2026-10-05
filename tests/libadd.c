/* rtld 验收夹具之一：共享库。
 *
 * 共享库的**全局符号默认进 .dynsym**（不需要 --export-dynamic），故 rtld 能按名找到 add。
 */
int add(int a, int b) { return a + b; }