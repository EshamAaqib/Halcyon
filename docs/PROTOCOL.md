# The `hfd` protocol

Reverse engineered, for compatibility, from the "HFD RGB Keyboard" vendor driver that ships with the LEOBOG
AMG65 (DeviceDriver.exe 1.0.3.1) and confirmed in part on hardware. The implementation is
`src/protocol/hfd.rs`, with byte-level tests in the same file and a byte-for-byte record of every command in
`tests/golden/hfd_python.json`.

Detailed specs, with the vendor function address behind every claim:

- [protocol/keymap.md](protocol/keymap.md): key remapping
- [protocol/macros.md](protocol/macros.md): macro storage and upload
- [protocol/perkey_settings.md](protocol/perkey_settings.md): per-key colours, keyboard settings, reset
- [protocol/displays.md](protocol/displays.md): LED matrix, LCD, clock and stats

Those specs were written from static analysis. Where hardware later disagreed, this page is right; see
[Confirmed on hardware](#confirmed-on-hardware).

## Devices and interfaces

| Connection | VID:PID | Control interface | Report |
|---|---|---|---|
| USB cable | 0C45:800A | usage page 0xFF68, usage 0x61 | 64 bytes + report ID 0 |
| USB cable, bulk data | 0C45:800A | usage page 0xFF67, usage 0x61 | 4096 bytes + report ID 0 |
| 2.4 GHz dongle | 05AC:024F | usage page 0xFF60, usage 0x61 | 32 bytes + report ID 0 |

All traffic is HID output and input reports. Feature reports are not used.

## Wired framing

Each operation is a transaction of 64-byte packets:

```
04 18                         begin
04 <cmd> .. [8..9] = count    header: number of data packets (or 4 KB blocks) that follow
<data>                        blocks usually end with AA 55
04 02                         end
04 F0                         commit (some commands)
```

Every packet is answered with `04 <cmd> 00 01`: byte 3 is 01 for OK, FF for busy.

## Dongle framing

One 32-byte frame per packet: `<cmd bytes> <data> <checksum>`, where the checksum is the 8-bit sum of the
report ID and the 31 bytes before it. The keyboard echoes the first three bytes. `02 00 00` asks for status
(byte 11 of the reply reads as battery percent, byte 12 as linked; inferred from observation).

## Commands

| Feature | Wired | Dongle |
|---|---|---|
| Lighting effect | `04 13`, 1 packet | `05 10 00` + block |
| Per-key colours | `04 23`, 8 packets, then lighting with mode 0x80 | mode 0x80, 21 frames `14 <len> <n>`, mode 0x80 again |
| Keymap | `04 11`, 8 packets | 19 frames `10 <len> <n>`; one key: `03 04 <index> <entry>` |
| Macros | `04 19`, `04 15`, image, `04 02` | frames `09 <len> <n>` |
| Settings | `04 17`, 1 packet | `07 10` |
| Clock | `04 28`, 1 packet | `0C 10` |
| Matrix animation | `04 33` (boot: `04 31`), 4 KB blocks | not available |
| LCD image | `04 72`, 4 KB blocks | not available |

### Lighting block (16 bytes)

| Byte | Field |
|---|---|
| 0 | effect: 1..19 animated effects, 20 = off, 0x80 = per-key colours |
| 1-3 | R, G, B |
| 8 | 1 = use the colour, 0 = rainbow |
| 9 | brightness 0..5 |
| 10 | speed 0..5 |
| 11 | direction 0..3 |
| 14-15 | AA 55 |

Effect numbers and names are in `devices/leobog-amg65.json`. The vendor app fills its list with
string IDs 200..219 in order, which gives effects 1..20.

### Tables

- **Keymap**: 128 entries of 4 bytes at `key_index * 4`, `AA 55` at 0x1FE. All zeros restores every key.
  Entries: `02 <modifier mask> <HID> 00` key, `03 <usage lo> <usage hi> 00` media, `01 01 <buttons> 00` mouse,
  `01 03 <01|FF> 00` scroll, `06 <macro slot> <mode> <count>` macro, `05 03 00 00` disabled.
- **Per-key colours**: entry for LED `i` at `i * 4` is `{i, R, G, B}`.
- **Macros**: 0xE00-byte image; 100 four-byte slot offsets, then bodies of four-byte events
  (`lo hi 00 50` delay in ms, `00 00 <HID> B0` key down, `00 00 <HID> 30` key up).
- **Matrix**: frame count, speed as `0x66 - speed`, then R, G, B per LED in device order (four 14×5 panels,
  then one 7×5), `AA 55`.
- **LCD**: 256-byte header (frame count, per-frame delay in 2 ms units, 0xFF padding), then 240×135 RGB565
  little-endian frames.

## Confirmed on hardware

- Lighting effects, off (effect 20) and the lighting block, over the dongle.
- Per-key colours over the dongle. The colours only appear when mode 0x80 is selected **again after** the
  table upload.
- Wired replies carry the OK flag in byte 3 (`04 18 00 01`), not byte 2.
- Matrix and LCD uploads over USB.
- The AMG65 has **one** LCD slot. The vendor app sets its screen count to 1; the byte sent in the `04 72`
  header and the clock block is the selected entry of its frame list, not a storage slot. Halcyon sends 1.
- `03 04 <index> <entry>` on the dongle writes a single key and is acknowledged.

## Not yet confirmed

Keymap, macros, settings and clock sync end to end; everything over USB except display uploads; the unit of
the matrix speed value; the meaning of the status reply's fields. The Fn layer cannot be programmed: the
vendor app never sends it.
