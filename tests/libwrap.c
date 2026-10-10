/* libwrap.c —— 依赖 libadd.so 的共享库（dlclose 依赖者保护夹具）。
 *
 * 存在的理由：rtld 的 dlclose 现在会 munmap 整段映射。若还有别的在世对象依赖它，
 * 那些对象里已写好的 GOT/PLT 槽就会指向已归还的帧。本库用来制造这个场景：
 * 它对 libadd.so 有真实的 DT_NEEDED，且真的会调用 add。
 */
extern int add(int a, int b);

int wrap_add(int a, int b) { return add(a, b); }
