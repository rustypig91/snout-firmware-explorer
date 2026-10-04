/* Third unit for dependency graph navigation, including reciprocal references. */
extern unsigned diagnose(unsigned);
extern unsigned ram_function(unsigned);
static const unsigned telemetry_bias[] = {2, 4, 6, 8};

__attribute__((noinline))
unsigned telemetry_scale(unsigned value) {
    return ram_function(value) + telemetry_bias[value & 3];
}

__attribute__((noinline))
unsigned telemetry_collect(unsigned value) {
    return diagnose(value) + telemetry_scale(value);
}
