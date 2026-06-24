/* freestanding static helper for the seccomp test: calls mount(2) and exits
 * 0 if allowed, 11 if EPERM (a seccomp trap), 12 on any other errno. */
#include <sys/syscall.h>

#if defined(__x86_64__)
static long sc(long n,long a,long b,long c,long d,long e){
  long r; register long r10 __asm__("r10")=d; register long r8 __asm__("r8")=e;
  __asm__ volatile("syscall":"=a"(r):"a"(n),"D"(a),"S"(b),"d"(c),"r"(r10),"r"(r8)
                   :"rcx","r11","memory");
  return r;
}
#elif defined(__aarch64__)
static long sc(long n,long a,long b,long c,long d,long e){
  register long x8 __asm__("x8")=n, x0 __asm__("x0")=a, x1 __asm__("x1")=b;
  register long x2 __asm__("x2")=c, x3 __asm__("x3")=d, x4 __asm__("x4")=e;
  __asm__ volatile("svc 0":"+r"(x0):"r"(x8),"r"(x1),"r"(x2),"r"(x3),"r"(x4):"memory","cc");
  return x0;
}
#else
#error "seccomp_probe: unsupported arch (need x86_64 or aarch64)"
#endif

void _start(void){
  long r = sc(SYS_mount,0,0,0,0,0);
  /* a raw syscall returns -errno; -1 is -EPERM, the seccomp trap. */
  long code = (r >= 0) ? 0 : (r == -1 ? 11 : 12);
  sc(SYS_exit_group,code,0,0,0,0);
  for(;;){}
}
