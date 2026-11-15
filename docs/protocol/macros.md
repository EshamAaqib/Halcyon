# Macros: storage layout and upload protocol (LEOBOG AMG65, DeviceDriver.exe v1.0.3.1)

Static analysis only. Tags: **[code]** = read directly from disassembly at the cited address;
**[inferred]** = deduced (UI strings, column order, naming) and not proven by code.

## 0. Not to be confused with

`04 20` (0x435d10, 0x444180) and dongle `0B 1C` (0x44436b) are **not macros**. Both build
4-byte records `{light_index, R, G, B}` from `CustomLightMode::GetKeyItems` (import 0x5867e4/0x586abc),
with optional brightness scaling `c*k>>8` for mode 7 (0x435e48..0x435e9e). That is the per-key colour
upload (see perkey_settings.md).

The real macro upload is:

| Path | Function | Notes |
|---|---|---|
| wired | **0x43dad0** | called from 0x44172b (macro page save handler 0x441600..) |
| dongle | **0x43e0d0** | called by 0x43dad0 at 0x43db0f when `[this+0x940]==2` (dongle mode) |

Both functions build the **same 0xE00-byte (3584 B) image** with identical code; only the transport differs.
PROTOCOL.md's "0x19, 0x15 / 09 1C ? (music?)" row is this macro upload.

## 1. Source data (vendor DB)

- Macro list: `0x402dd0` = `SELECT * FROM t_macro_data` (vector of 0x14-byte structs, `+0` = macro_id). [code: call sites 0x43dbd8/0x43e19b; struct size freed at 0x43df62 `push 0x14`]
- Records of a macro: `0x4032b0(macro_id)` = `SELECT * FROM t_macrorecord WHERE macro_id=? ORDER BY record_index ASC`.
  Record struct (0x18 bytes): `+8 type`, `+0xC record_index`, `+0x10 value`, `+0x14 desc` (CString). [code for +8/+0x10 use at 0x43dd3f/0x43dd4d; field mapping from the column order is inferred]
- `play_times`, `delay_type`, `delay_time` in t_macro_data are **never read by the upload**: only `+0 macro_id`
  is used (0x43dc7e). They are recording-time UI settings ("Recording Delay: No Delay / Default / ... ms", lang 517-520). [code + inferred]

Record `type` values:

| type | meaning | value |
|---|---|---|
| 1 | delay | milliseconds (16-bit used) |
| 2 | key down | Windows VK code |
| 3 | key up | Windows VK code |
| 4 | mouse button down | 1 / 2 / 3 |
| 5 | mouse button up | 1 / 2 / 3 |

The meanings come from the encoder below [code]. Mouse value 1=Left, 2=Middle, 3=Right is [inferred] from the UI string order 507/508/509
("left/Middle/Right Mouse Button") together with the HID bit each value is mapped to (1→0x01, 2→0x04, 3→0x02, which are the standard HID L/M/R bits).

## 2. Macro image layout (0xE00 bytes, memset 0 first)  [code 0x43db74..0x43df1c]

```
0x000 .. 0x18F   index table: 100 slots x 4 bytes
0x190 .. 0xDFF   macro bodies, packed back to back in slot order
```

### 2.1 Index table entry (slot i = i-th row of SELECT * FROM t_macro_data)
- Macro with ≥1 record: `[i*4+0] = offset & 0xFF`, `[i*4+1] = offset >> 8`, `[i*4+2..3] = 00 00` (offset is absolute in the image). [code 0x43dc99/0x43dca5]
- Macro that exists but has 0 records: `FF FF FF FF`. [code 0x43df36]
- Slots past the number of macros: left `00 00 00 00` (memset). [code]
- Max 100 slots. The first body always starts at 0x190 (`mov ebx,0x190` at 0x43db94). The code does not bound-check the slot count; anything over 100 would overwrite bodies.

### 2.2 Macro body
```
+0  u16 LE  event_count   (written after encoding, 0x43df05/0x43df0f)
+2  6 bytes 00            (never written)
+8  event_count x 4-byte events
```
The next body starts at `offset + 8 + 4*event_count` (0x43df16..0x43df1c).

### 2.3 Event encoding (4 bytes)  [code 0x43dd3f..0x43de7f]
```
byte0  byte1  byte2  byte3(flags)
```
| Event | b0 | b1 | b2 | b3 |
|---|---|---|---|---|
| delay N ms | N & 0xFF | N >> 8 | 00 | **0x50** |
| key down | 00 | 00 | HID usage | **0xB0** |
| key up | 00 | 00 | HID usage | **0x30** |
| mouse down | 00 | 00 | button bit | **0x90** |
| mouse up | 00 | 00 | button bit | **0x10** |

- Flag byte, read from the values: bit7 = press(1)/release(0). Bits 4-6 = class: 1 = mouse, 3 = keyboard, 5 = delay. [inferred interpretation; the values are from code]
- Button bit: 0x01 = left, 0x04 = middle, 0x02 = right (HID boot-mouse bits). [code; L/M/R naming inferred]
- Delay is a 16-bit LE value in ms (units from lang 520 "ms"), so the maximum is 65535 ms. The code takes the low 2 bytes (`+0x10`, `+0x11`) of the stored int. [code; unit inferred]
- **Implicit delay:** if a non-delay event follows another non-delay event in the same macro, the encoder inserts
  `0A 00 00 50` (10 ms) before it. No delay is inserted before the first event or after a delay. Inserted delays count in event_count. [code 0x43dd6f..0x43ddc2]
- Records of type 4/5 with a value outside 1..3, or of unknown type: event_count is still incremented, but the slot is left `00 00 00 00` (a vendor bug). [code 0x43de8a]

### 2.4 VK → HID conversion (0x46a820)  [code]
A..Z (0x41-0x5A) → 0x04-0x1D; '1'..'9' (0x31-0x39) → 0x1E-0x26; plus this compare chain:
```
0x30->27 0D->28 1B->29 08->2A 09->2B 20->2C BD->2D BB->2E DB->2F DD->30 DC->31 29A->32 BA->33 DE->34
C0->35 BC->36 BE->37 BF->38 14->39 70..7B(F1-F12)->3A..45 7C..87(F13-F24)->68..73 2C->46 91->47 13->48
2D->49 24->4A 21->4B 2E->4C 23->4D 22->4E 27->4F 25->50 28->51 26->52 90->53 6F->54 6A->55 6D->56
6B->57 6C->58 61..69(Num1-9)->59..61 60->62 6E->63 A2->E0 A0->E1 A4->E2 5B->E3 A3->E4 A1->E5 A5->E6
5C->E7 5D->65 FE->AF E2->64 12D->EC   (anything else -> 0)
```
Note: the left/right modifiers use the VK_L*/VK_R* codes (A0-A5). A replacement app can write HID usages directly.

### 2.5 Capacity
- Hard limit: 0xE00 bytes in total; 3184 bytes for bodies. The encoder bound-checks only the implicit-delay write (`cmp ...,0xe00` at 0x43dd77).
- Upload refuses to send when `(total_records + 2*macro_count)*4 > 0xA8C` (2700) (0x43db2e..0x43db4d, also 0x43e10c..).
  Here `total_records` = `select count(*) FROM t_macrorecord` (0x4035d0) and `macro_count` = item count of the macro list box `[this+0x527d4]` (HFDListBox::GetItemCount).
- The UI's "remaining space" is `0x145*2 - 2*macro_count - total_records` = 650 - 2·macros - records (0x440c80..0x440c8b).
  Below 10 it shows lang 523/560 ("not enough space ..."). So each event costs 1 unit (4 B) and each macro header costs 2 units (8 B). [code]
  Implicit 10 ms delays are not counted by these checks.
- Practical vendor limit: about 650-675 events across all macros. Device-side limit: about 796 4-byte units after the table. [inferred]

## 3. Wired upload sequence (0x43dad0)  [code]

Each line is one 64-byte payload sent with send65 (0x467f50: report ID 0 is prepended, a 65-byte hid_write, then
hid_read_timeout 200 ms; success if reply byte[3] (=payload[2] after the report ID) == 1, retry once on 0xFF).

```
1. 04 19 00 00 00 00 00 00 00 ... (64 B)        begin-macro   (0x43db63, send65 len 0x41)
2. 04 15 00 00 00 00 00 00 NN 00 ... (64 B)     header, byte8 = NN (0x43e01a/0x43e015)
   Sleep(30 ms)                                 (0x43e03e)
3. send65(image, NN*64 + 64)                    data (0x43e050)
   -> send65 sends (len>>6) = NN+1 chunks of 64 bytes, each followed by a read (200 ms) and Sleep(2)
   Sleep(30 ms)
4. 04 02 00 ... (64 B)                          end/apply (0x43e069)
```
There is no `04 18` begin and no `04 F0` commit in this transaction.

Packet count: `total = end offset of the last body` (0x190 + Σ(8+4·events)).
`NN = total/64 + 1` if `total % 64 == 0`, otherwise `total/64 + 2` (0x43dff2..0x43e00f).
`AA 55` is written at image[NN*64-2 .. NN*64-1], the last 2 bytes of packet NN-1 (0x43e029).
The vendor then sends **one extra all-zero packet** (index NN) because of the `+64` in the length. A replacement
can probably send exactly NN packets. [inferred: harmless trailing packet, untested]

## 4. Dongle upload sequence (0x43e0d0)  [code]

The same image is split into 28-byte slices, one per 33-byte dongle33 frame (0x468790). There are **no begin/end frames**.
Each frame is followed by Sleep(2 ms) (0x43e675).

```
q = total / 28 ; r = total % 28
F = q + 1 + (r != 0)                          number of frames (0x43e5c0..0x43e5f6)
image[F*28-2 .. F*28-1] = AA 55               (0x43e603)

frame j (j = 0..F-1), 33 bytes on the wire:
  [0]      00               report ID
  [1]      09               command
  [2]      1C               (= 28, data length); on the LAST frame only it is overwritten with (total - q*28) & 0xFF = r   (0x43e633..0x43e64f)
  [3]      j                frame index (u8)                               (0x43e630)
  [4..31]  image[j*28 .. j*28+27]                                         (0x43e652..0x43e66b)
  [32]     checksum = (sum of bytes [0..31]) & 0xFF                       (0x468816/0x46881c; bytes 0x21..0x40 are zero)
```
The last frame is always past the data. It holds only zeros plus `AA 55` in its last two data bytes (frame[30..31]),
and frame[2] = r. If r = 0, that byte is 0x00. [code; the firmware meaning of byte2 on the last frame is inferred as "length of the final partial slice"]
dongle33 writes 0x21 bytes and reads the echo (see PROTOCOL.md). The macro function ignores the return value.

## 5. Play mode / repeat: stored in the keymap, not in the macro image

The macro image has no play-mode field. The mode is part of the key binding:

- Keymap upload 0x41ad70, binding type 3 (macro), at 0x41b004..0x41b0b2. The 4-byte keymap entry is
  `06 <slot> <macro2value> <macro3value>`. [code]
  `<slot>` = index of the macro's macro_id in the `SELECT * FROM t_macro_data` list (the same order as the image's index table). [code 0x41b073..0x41b0ab]
- Key-assign dialog OK handler 0x4140c0, macro branch at 0x41412b..0x414275 [code]:
  - macrovalue (`+0x1c`) = macro_id from combo `[dlg+0x38c]`
  - macro2value (`+0x20`) = play-mode combo `[dlg+0x398]` item data, filled at 0x412586..0x41266d:
    **0 = "Play once" (lang 196), 1 = "Play multiple times" (197), 2 = "Stop when key again" (198)**
  - macro3value (`+0x24`) = loop count from edit `[dlg+0x39c]`, used only when mode == 1. Values < 1 become 1 and values > 255 become 255 (0x41423f..0x414267).
    For other modes it stays 0 (the struct is zero-initialised; the default is inferred).
- No "while held" mode exists in this app. The only modes are once / N times / toggle until pressed again. [code: only 3 combo items]
  How the firmware handles a fourth value (e.g. 3) is unknown.

Keymap entry examples [code-derived]:
```
06 00 00 00   slot 0, play once
06 00 01 05   slot 0, play 5 times
06 00 02 00   slot 0, repeat until the key is pressed again
```

## 6. Example: one macro (slot 0) "press A, 50 ms, release A"

DB records: (2,0x41) (1,50) (3,0x41). There is no implicit delay because the middle event is a delay.

Image (only non-zero bytes shown):
```
0x000: 90 01 00 00                         slot0 -> offset 0x190
0x190: 03 00 00 00 00 00 00 00             event_count = 3
0x198: 00 00 04 B0                         A down (HID 0x04)
0x19C: 32 00 00 50                         delay 50 ms
0x1A0: 00 00 04 30                         A up
total = 0x1A4 (420)
```
Without the explicit delay (A down, A up) the encoder would emit `00 00 04 B0 | 0A 00 00 50 | 00 00 04 30`, count = 3.

### Wired (NN = 420/64 = 6 rem 36 → NN = 8)
```
04 19 00 00 ... 00
04 15 00 00 00 00 00 00 08 00 ... 00
P0: 90 01 00 00 00 ... 00
P1..P5: 64 x 00
P6: 00x16 03 00 00 00 00 00 00 00 00 00 04 B0 32 00 00 50 00 00 04 30 00x28
P7: 00x62 AA 55
P8: 64 x 00          (vendor's extra packet)
04 02 00 ... 00
```
(This lands at 8 packets = 512 B, but only by coincidence. NN grows with content, up to 57.)

### Dongle (q = 15, r = 0 → F = 16 frames), 33 bytes each including report ID and checksum
```
D00 00 09 1c 00 90 01 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 b6
D01 00 09 1c 01 (28 x 00) 26
D02..D13: 00 09 1c jj (28 x 00) cs   with cs = 0x25 + jj
D14 00 09 1c 0e 00 00 00 00 00 00 00 00 03 00 00 00 00 00 00 00 00 00 04 b0 32 00 00 50 00 00 04 30 a0
D15 00 09 00 0f 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 aa 55 17
```
These bytes come from a re-implementation of 0x43dad0/0x43e0d0; `src/protocol/hfd.rs` produces the same.

Then bind a key: keymap entry `06 00 00 00` (play once), sent with the 04 11 keymap upload (see keymap.md).

## 7. Open questions / risks
- Unconfirmed: whether the firmware needs the 6 zero bytes at body+2..+7, or uses them (e.g. for repeat).
  The vendor always sends zeros.
- Unconfirmed: whether the trailing zero packet (wired) and the r-in-byte2 last frame (dongle) are required.
- No macro *read-back* command was seen. None of the transport call sites reads macros, but this was not searched exhaustively. The DB appears to be the source of truth.
- The upload always rewrites the full table (all macros at once). There is no per-macro write.
