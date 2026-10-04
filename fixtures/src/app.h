#ifndef FIXTURE_APP_H
#define FIXTURE_APP_H

/* A small, freestanding sensor monitor with simulated input and output. */
enum {
    SAMPLE_WINDOW = 16,
    TELEMETRY_QUEUE_CAPACITY = 4,
    FAULT_SENSOR_RANGE = 1u << 0,
    FAULT_QUEUE_FULL = 1u << 1,
    FAULT_PACKET_CHECKSUM = 1u << 2,
    FAULT_BACKPRESSURE = 1u << 3,
};

typedef struct {
    unsigned sample_period_ticks;
    unsigned report_interval_ticks;
    unsigned alarm_threshold;
    unsigned checksum_seed;
} DeviceConfig;

typedef struct {
    unsigned sequence;
    unsigned value;
    unsigned status;
    unsigned checksum;
} TelemetryPacket;

/* Configuration periods are powers of two so scheduling needs no division. */
const DeviceConfig *config_get(void);
unsigned config_baudrate(void);

unsigned config_alarm_threshold(unsigned profile);
unsigned config_checksum_seed(void);

void sensor_init(unsigned seed);
unsigned sensor_sample(unsigned tick);

unsigned diagnose(unsigned value);
void diagnostics_record_fault(unsigned fault);
unsigned diagnostics_status(void);

unsigned telemetry_scale(unsigned value);
unsigned telemetry_collect(unsigned value);
unsigned telemetry_checksum(const TelemetryPacket *packet);

void transport_init(void);
unsigned transport_enqueue(const TelemetryPacket *packet);
unsigned transport_pending(void);
void transport_flush(void);

/* RAM-resident calibration and optional board hook also exercise ELF analysis. */
unsigned ram_function(unsigned value);
void weak_callback(void);

#endif
