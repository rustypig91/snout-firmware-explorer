#include "app.h"

static unsigned packet_sequence;
static const unsigned telemetry_bias[] = {2, 4, 6, 8};

__attribute__((noinline)) unsigned telemetry_scale(unsigned value)
{
    return (ram_function(value) + telemetry_bias[value & 3]) >> 2;
}

unsigned telemetry_checksum(const TelemetryPacket *packet)
{
    unsigned checksum = config_checksum_seed();
    checksum ^= packet->sequence;
    checksum = (checksum << 5) | (checksum >> 27);
    checksum ^= packet->value;
    checksum = (checksum << 5) | (checksum >> 27);
    return checksum ^ packet->status;
}

__attribute__((noinline)) unsigned telemetry_collect(unsigned value)
{
    TelemetryPacket packet;

    packet.sequence = packet_sequence++;
    packet.value = telemetry_scale(value);
    packet.status = diagnose(value) | diagnostics_status();
    packet.checksum = telemetry_checksum(&packet);

    if (!transport_enqueue(&packet)) {
        diagnostics_record_fault(FAULT_QUEUE_FULL);
        return 0;
    }

    return packet.sequence + 1;
}
