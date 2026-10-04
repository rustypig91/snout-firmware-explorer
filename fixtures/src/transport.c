#include "app.h"

static TelemetryPacket pending_packets[TELEMETRY_QUEUE_CAPACITY];
static unsigned read_index;
static unsigned write_index;
static unsigned pending_count;
static volatile unsigned last_sent_checksum;

void transport_init(void)
{
    read_index = 0;
    write_index = 0;
    pending_count = 0;
    last_sent_checksum = config_baudrate();
}

unsigned transport_pending(void)
{
    return pending_count;
}

unsigned transport_enqueue(const TelemetryPacket *packet)
{
    if (pending_count == TELEMETRY_QUEUE_CAPACITY) {
        return 0;
    }

    pending_packets[write_index] = *packet;
    write_index = (write_index + 1) & (TELEMETRY_QUEUE_CAPACITY - 1);
    pending_count++;
    return 1;
}

void transport_flush(void)
{
    while (pending_count != 0) {
        const TelemetryPacket *packet = &pending_packets[read_index];

        if (telemetry_checksum(packet) != packet->checksum) {
            diagnostics_record_fault(FAULT_PACKET_CHECKSUM);
        } else {
            /* Stand-in for a UART register write; keep an observable result. */
            last_sent_checksum = packet->checksum;
        }

        read_index = (read_index + 1) & (TELEMETRY_QUEUE_CAPACITY - 1);
        pending_count--;
    }
}
