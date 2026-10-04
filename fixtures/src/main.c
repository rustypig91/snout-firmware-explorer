#include "app.h"

#ifndef EXTRA
#define EXTRA 0
#endif

/* EXTRA reserves eight more sample slots in the comparison build. */
volatile unsigned initialized = 42;
volatile unsigned samples[SAMPLE_WINDOW + EXTRA];

static volatile unsigned private_state;
static unsigned tick_count;
static unsigned sample_index;

__attribute__((section(".ram_code"), noinline)) unsigned ram_function(unsigned value)
{
    return value * 3 + 1;
}

__attribute__((weak, noinline)) void weak_callback(void)
{
    private_state++;
}

/* Retain a weak alias and a mangled symbol as explicit analysis test cases. */
void callback_alias(void) __attribute__((weak, alias("weak_callback")));
__attribute__((noinline)) unsigned cpp_function(unsigned) __asm__("_Z12cpp_functionj");

unsigned cpp_function(unsigned value)
{
    return value + initialized;
}

static void application_init(void)
{
    tick_count = 0;
    sample_index = 0;
    sensor_init(initialized);
    transport_init();

    for (unsigned index = 0; index < SAMPLE_WINDOW; index++) {
        samples[index] = 0;
    }
}

static void application_step(void)
{
    const DeviceConfig *config = config_get();
    tick_count++;

    if ((tick_count & (config->sample_period_ticks - 1)) == 0) {
        unsigned value = sensor_sample(tick_count);
        samples[sample_index] = value;
        sample_index = (sample_index + 1) & (SAMPLE_WINDOW - 1);

        if (diagnose(value) != 0) {
            weak_callback();
        }

        if ((tick_count & (config->report_interval_ticks - 1)) == 0) {
            telemetry_collect(value);
        }
    }

    /* A real board would drain the queue from a UART/DMA completion handler. */
    transport_flush();
}

int main(void)
{
    application_init();

    for (;;) {
        application_step();
    }
}

/* Analysis entry point only: a board startup would also initialize RAM. */
void Reset_Handler(void)
{
    main();
}
