/* Kept in the compiler matrix to exercise wrapped custom TI output names. */
__attribute__((section(".sensor_calibration_code"), noinline)) unsigned
sensor_calibration_bias(unsigned value)
{
    return value + 7;
}
