/*
 * VeridianOS -- bluez-hci-bridge.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * HCI bridge implementation.  Communicates with the VeridianOS kernel
 * Bluetooth HCI driver via /dev/bluetooth/hci0 using standard read/write
 * and ioctl operations.
 *
 * Packet format follows the HCI UART (H4) transport:
 *   Command: [0x01] [opcode LE16] [param_len U8] [params...]
 *   Event:   [0x04] [event_code U8] [param_len U8] [params...]
 */

#include "bluez-hci-bridge.h"

#include <QDebug>

#include <cstring>
#include <cstdlib>

#include <unistd.h>
#include <fcntl.h>
#include <sys/ioctl.h>
#include <errno.h>

/* ========================================================================= */
/* Constants                                                                 */
/* ========================================================================= */

/* H4 packet type indicators */
static const uint8_t H4_COMMAND = 0x01;
static const uint8_t H4_EVENT   = 0x04;

/* Maximum HCI packet sizes */
static const uint32_t HCI_MAX_COMMAND_LEN = 259;   /* 1 + 2 + 1 + 255 */
static const uint32_t HCI_MAX_EVENT_LEN   = 259;   /* 1 + 1 + 1 + 255 + 1 (H4) */

/* HCI status codes */
static const uint8_t HCI_STATUS_SUCCESS          = 0x00;
static const uint8_t HCI_STATUS_UNKNOWN_COMMAND   = 0x01;
static const uint8_t HCI_STATUS_NO_CONNECTION     = 0x02;
static const uint8_t HCI_STATUS_HW_FAILURE        = 0x03;
static const uint8_t HCI_STATUS_PAGE_TIMEOUT      = 0x04;
static const uint8_t HCI_STATUS_AUTH_FAILURE       = 0x05;
static const uint8_t HCI_STATUS_PIN_MISSING        = 0x06;
static const uint8_t HCI_STATUS_MEMORY_EXCEEDED    = 0x07;
static const uint8_t HCI_STATUS_CONN_TIMEOUT       = 0x08;

/* Inquiry LAP: General/Unlimited Inquiry Access Code (GIAC) */
static const uint8_t GIAC_LAP[3] = { 0x33, 0x8B, 0x9E };

/* ========================================================================= */
/* HciBridge internal state                                                  */
/* ========================================================================= */

struct HciBridge {
    int fd;                     /* file descriptor for /dev/bluetooth/hciN */
    bool open;
    char dev_path[128];

    /* Receive buffer for event reassembly */
    uint8_t rx_buf[HCI_MAX_EVENT_LEN];
    uint32_t rx_len;
};

/* ========================================================================= */
/* Helper: HCI status code to string                                         */
/* ========================================================================= */

static const char *hci_status_str(uint8_t status)
{
    switch (status) {
    case HCI_STATUS_SUCCESS:        return "Success";
    case HCI_STATUS_UNKNOWN_COMMAND: return "Unknown HCI Command";
    case HCI_STATUS_NO_CONNECTION:  return "No Connection";
    case HCI_STATUS_HW_FAILURE:     return "Hardware Failure";
    case HCI_STATUS_PAGE_TIMEOUT:   return "Page Timeout";
    case HCI_STATUS_AUTH_FAILURE:    return "Authentication Failure";
    case HCI_STATUS_PIN_MISSING:     return "PIN or Key Missing";
    case HCI_STATUS_MEMORY_EXCEEDED: return "Memory Capacity Exceeded";
    case HCI_STATUS_CONN_TIMEOUT:    return "Connection Timeout";
    default:                         return "Unknown Error";
    }
}

/* ========================================================================= */
/* Lifecycle                                                                 */
/* ========================================================================= */

HciBridge *hci_bridge_new(void)
{
    HciBridge *bridge = new HciBridge;
    if (!bridge)
        return nullptr;

    memset(bridge, 0, sizeof(*bridge));
    bridge->fd = -1;
    bridge->open = false;

    return bridge;
}

bool hci_bridge_open(HciBridge *bridge, const char *dev_path)
{
    if (!bridge || !dev_path)
        return false;

    if (bridge->open) {
        qWarning("HciBridge: already open on %s", bridge->dev_path);
        return false;
    }

    strncpy(bridge->dev_path, dev_path, sizeof(bridge->dev_path) - 1);
    bridge->dev_path[sizeof(bridge->dev_path) - 1] = '\0';

    bridge->fd = open(dev_path, O_RDWR);
    if (bridge->fd < 0) {
        qWarning("HciBridge: cannot open %s: %s", dev_path, strerror(errno));
        return false;
    }

    bridge->open = true;
    bridge->rx_len = 0;

    qDebug("HciBridge: opened %s (fd=%d)", dev_path, bridge->fd);
    return true;
}

void hci_bridge_close(HciBridge *bridge)
{
    if (!bridge || !bridge->open)
        return;

    close(bridge->fd);
    bridge->fd = -1;
    bridge->open = false;
    bridge->rx_len = 0;

    qDebug("HciBridge: closed %s", bridge->dev_path);
}

void hci_bridge_destroy(HciBridge *bridge)
{
    if (!bridge)
        return;

    if (bridge->open)
        hci_bridge_close(bridge);

    delete bridge;
}

/* ========================================================================= */
/* Command / Event                                                           */
/* ========================================================================= */

bool hci_bridge_send_command(HciBridge *bridge, uint16_t opcode,
                              const uint8_t *params, uint8_t len)
{
    if (!bridge || !bridge->open)
        return false;

    if (len > 255) {
        qWarning("HciBridge: command params too long (%u)", len);
        return false;
    }

    /* Build H4 command packet */
    uint8_t pkt[HCI_MAX_COMMAND_LEN];
    uint32_t pkt_len = 0;

    pkt[pkt_len++] = H4_COMMAND;
    pkt[pkt_len++] = (uint8_t)(opcode & 0xFF);         /* opcode low byte */
    pkt[pkt_len++] = (uint8_t)((opcode >> 8) & 0xFF);  /* opcode high byte */
    pkt[pkt_len++] = len;                               /* parameter length */

    if (len > 0 && params) {
        memcpy(&pkt[pkt_len], params, len);
        pkt_len += len;
    }

    ssize_t written = write(bridge->fd, pkt, pkt_len);
    if (written < 0) {
        qWarning("HciBridge: write failed: %s", strerror(errno));
        return false;
    }

    if ((uint32_t)written != pkt_len) {
        qWarning("HciBridge: short write (%zd/%u)", written, pkt_len);
        return false;
    }

    qDebug("HciBridge: sent command opcode=0x%04X param_len=%u", opcode, len);
    return true;
}

int hci_bridge_recv_event(HciBridge *bridge, uint8_t *buf, uint32_t maxlen)
{
    if (!bridge || !bridge->open || !buf || maxlen < 3)
        return -1;

    /* Read from the device -- the kernel delivers complete HCI events.
     * The first byte is the H4 event indicator (0x04), followed by
     * event_code, param_len, and parameters. */
    uint8_t raw[HCI_MAX_EVENT_LEN];
    ssize_t nread = read(bridge->fd, raw, sizeof(raw));

    if (nread < 0) {
        if (errno == EAGAIN || errno == EINTR)
            return 0;  /* no event available */
        qWarning("HciBridge: read failed: %s", strerror(errno));
        return -1;
    }

    if (nread < 3) {
        qWarning("HciBridge: short read (%zd bytes)", nread);
        return -1;
    }

    /* Verify H4 event indicator */
    uint32_t offset = 0;
    if (raw[0] == H4_EVENT) {
        offset = 1;  /* skip H4 type byte */
    }

    /* Copy event_code + param_len + params into output buffer */
    uint32_t event_len = (uint32_t)(nread - offset);
    if (event_len > maxlen)
        event_len = maxlen;

    memcpy(buf, &raw[offset], event_len);

    uint8_t event_code = buf[0];
    uint8_t param_len = (event_len >= 2) ? buf[1] : 0;

    qDebug("HciBridge: received event code=0x%02X param_len=%u",
           event_code, param_len);

    return (int)event_len;
}

/* ========================================================================= */
/* Convenience wrappers                                                      */
/* ========================================================================= */

bool hci_bridge_start_inquiry(HciBridge *bridge, uint8_t duration_secs)
{
    if (!bridge)
        return false;

    /* Clamp duration to 1-30 seconds */
    if (duration_secs < 1) duration_secs = 1;
    if (duration_secs > 30) duration_secs = 30;

    /* HCI_Inquiry parameters:
     * LAP (3 bytes) + inquiry_length (1 byte, units of 1.28s) + num_responses (1 byte)
     */
    uint8_t inquiry_length = (duration_secs * 10 + 12) / 13;  /* convert to 1.28s units */
    if (inquiry_length < 1) inquiry_length = 1;
    if (inquiry_length > 0x30) inquiry_length = 0x30;

    uint8_t params[5];
    params[0] = GIAC_LAP[0];
    params[1] = GIAC_LAP[1];
    params[2] = GIAC_LAP[2];
    params[3] = inquiry_length;
    params[4] = 0x00;  /* unlimited responses */

    qDebug("HciBridge: starting inquiry for %u seconds (length=%u)",
           duration_secs, inquiry_length);

    return hci_bridge_send_command(bridge, HCI_OP_INQUIRY, params, 5);
}

bool hci_bridge_cancel_inquiry(HciBridge *bridge)
{
    if (!bridge)
        return false;

    qDebug("HciBridge: cancelling inquiry");
    return hci_bridge_send_command(bridge, HCI_OP_INQUIRY_CANCEL, nullptr, 0);
}

bool hci_bridge_get_local_address(HciBridge *bridge, uint8_t *addr_out)
{
    if (!bridge || !addr_out)
        return false;

    /* Send HCI_Read_BD_ADDR */
    if (!hci_bridge_send_command(bridge, HCI_OP_READ_BD_ADDR, nullptr, 0))
        return false;

    /* Wait for Command Complete event */
    uint8_t evt_buf[HCI_MAX_EVENT_LEN];
    int evt_len = hci_bridge_recv_event(bridge, evt_buf, sizeof(evt_buf));
    if (evt_len < 2)
        return false;

    uint8_t event_code = evt_buf[0];
    if (event_code != HCI_EVT_COMMAND_COMPLETE) {
        qWarning("HciBridge: expected CommandComplete, got 0x%02X", event_code);
        return false;
    }

    /* CommandComplete params: [num_hci_cmd_pkts(1)] [opcode(2)] [status(1)] [addr(6)] */
    uint8_t param_len = evt_buf[1];
    if (param_len < 10)
        return false;

    uint8_t status = evt_buf[2 + 3];  /* skip num_pkts(1) + opcode(2) */
    if (status != HCI_STATUS_SUCCESS) {
        qWarning("HciBridge: Read_BD_ADDR failed: %s", hci_status_str(status));
        return false;
    }

    /* BD_ADDR is at offset 2 + 4 = 6 in the event buffer */
    memcpy(addr_out, &evt_buf[2 + 4], 6);

    qDebug("HciBridge: local address %02X:%02X:%02X:%02X:%02X:%02X",
           addr_out[5], addr_out[4], addr_out[3],
           addr_out[2], addr_out[1], addr_out[0]);

    return true;
}

bool hci_bridge_get_local_name(HciBridge *bridge, char *name_out,
                                uint32_t maxlen)
{
    if (!bridge || !name_out || maxlen < 2)
        return false;

    /* Send HCI_Read_Local_Name */
    if (!hci_bridge_send_command(bridge, HCI_OP_READ_LOCAL_NAME, nullptr, 0))
        return false;

    /* Wait for Command Complete event */
    uint8_t evt_buf[HCI_MAX_EVENT_LEN];
    int evt_len = hci_bridge_recv_event(bridge, evt_buf, sizeof(evt_buf));
    if (evt_len < 2)
        return false;

    uint8_t event_code = evt_buf[0];
    if (event_code != HCI_EVT_COMMAND_COMPLETE) {
        qWarning("HciBridge: expected CommandComplete, got 0x%02X", event_code);
        return false;
    }

    /* CommandComplete params: [num_pkts(1)] [opcode(2)] [status(1)] [name(248)] */
    uint8_t param_len = evt_buf[1];
    if (param_len < 5)
        return false;

    uint8_t status = evt_buf[2 + 3];
    if (status != HCI_STATUS_SUCCESS) {
        qWarning("HciBridge: Read_Local_Name failed: %s", hci_status_str(status));
        return false;
    }

    /* Name starts at offset 2 + 4 = 6, up to 248 bytes, NUL-terminated */
    const char *name_src = (const char *)&evt_buf[2 + 4];
    uint32_t name_avail = (uint32_t)(evt_len - 6);
    if (name_avail > 248)
        name_avail = 248;

    uint32_t copy_len = (name_avail < maxlen - 1) ? name_avail : (maxlen - 1);
    memcpy(name_out, name_src, copy_len);
    name_out[copy_len] = '\0';

    /* Trim at first NUL within the name */
    for (uint32_t i = 0; i < copy_len; ++i) {
        if (name_out[i] == '\0')
            break;
    }

    qDebug("HciBridge: local name '%s'", name_out);
    return true;
}
