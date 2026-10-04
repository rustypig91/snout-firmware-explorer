#include "app.h"

/* Seeded to represent a persisted fault counter and exercise initialized RAM. */
static volatile unsigned diagnostic_count = 7;
static unsigned active_faults;
static const unsigned limits[] = {12, 24, 48, 96};

__attribute__((section(".unusual_constants"), used))
const unsigned signature[] = {0x12345678, 0xabcdef01};

void diagnostics_record_fault(unsigned fault)
{
    active_faults |= fault;
    diagnostic_count++;
}

unsigned diagnostics_status(void)
{
    return active_faults;
}

__attribute__((noinline)) unsigned diagnose(unsigned value)
{
    volatile unsigned scratch[8];
    unsigned profile = value & 3;
    unsigned fault = 0;

    scratch[0] = telemetry_scale(value);
    scratch[1] = config_alarm_threshold(profile);
    scratch[2] = limits[profile];

    if (scratch[0] > scratch[1] + scratch[2]) {
        fault |= FAULT_SENSOR_RANGE;
    }

    if (transport_pending() == TELEMETRY_QUEUE_CAPACITY) {
        fault |= FAULT_BACKPRESSURE;
    }

    if (fault != 0) {
        diagnostics_record_fault(fault);
    }

    return fault;
}
