/* Tiny Cortex-M analysis fixture, not a board startup implementation. */
#ifndef EXTRA
#define EXTRA 0
#endif
volatile unsigned initialized = 42;
volatile unsigned samples[16 + EXTRA];
const unsigned baudrate_table[] = {9600, 19200, 38400, 115200};
static volatile unsigned private_state;
extern unsigned diagnose(unsigned);
__attribute__((section(".ram_code"), noinline))
unsigned ram_function(unsigned value) { return value * 3 + 1; }
__attribute__((weak, noinline))
void weak_callback(void) { private_state++; }
void callback_alias(void) __attribute__((weak, alias("weak_callback")));
__attribute__((noinline)) unsigned cpp_function(unsigned) __asm__("_Z12cpp_functionj");
unsigned cpp_function(unsigned value) { return value + initialized; }
void Reset_Handler(void) {
    samples[0] = diagnose(ram_function(initialized));
    weak_callback();
    for (;;) { private_state++; }
}
