# Key remapping protocol (LEOBOG AMG65, DeviceDriver.exe v1.0.3.1)

Static analysis only. All addresses are VAs in DeviceDriver.exe (image base 0x400000).
Tags: **[code]** means read directly from disassembly. **[inf]** means inferred (semantics or naming not proven by code).

---------------------------------------------------------------------------------------------------

## 1. Summary

* The keymap is a flat **512-byte table of 128 x 4-byte entries**. Entry *i* sits at offset `key_index*4`,
  where `key_index` comes from `KeyboardLayout.xml`. Bytes `0x1FE..0x1FF` hold the signature `AA 55`. **[code]**
* Wired: `04 18` begin, then `04 11` with `[8]=08`, then 8 x 64-byte data packets (the table), then `04 02`, then `04 F0`. **[code]** 0x41ad70
* Dongle, full table: 19 frames `10 <len> <seq> <28 data bytes>` with `len=0x1C`, except the last frame where `len=0x08`.
  No begin/end frames. **[code]** 0x41b8e0
* Dongle, single key: one frame `03 04 <key_index> <4-byte entry>`. **[code]** 0x41bb30
* **The Fn layer is never sent to the keyboard.** Every encoder loads `fnlayer = 0` only. Fn-layer edits stay in the local SQLite DB. **[code]** (see section 7)
* The code has **no read-back command for the keymap**, and no dedicated device-side "factory reset" command.
  "Reset keyboard data" and "Restore factory settings" both work by re-sending tables built from the local DB. **[code]** (section 9)

---------------------------------------------------------------------------------------------------

## 2. Where the data comes from

The SQLite table `t_key_data` is loaded by `0x4074d0(profile, fnlayer, out_vec)`
(`SELECT * FROM t_key_data WHERE profile = ? and fnlayer = ?`, at 0x59b0e8). The record struct is filled at 0x4075f0..0x407705. **[code]**

| off | column | meaning |
|---|---|---|
| +0x00 | key_id | |
| +0x04 | profile | 1-based configuration number (built-in profile 2 = "Mac", profile 3 = "Dvorak"; see 0x407770 and 0x407930) |
| +0x08 | keyvalue | **physical key, given as the `code=` HID usage from KeyboardLayout.xml** (for example Esc=0x29, Fn=0xAF) |
| +0x0C | fnlayer | 0 = base layer, 1 = Fn layer |
| +0x10 | layoutkeyvalue | |
| +0x14 | layoutkeydesc | string |
| +0x18 | macrotype | function type, see section 4 |
| +0x1C | macrovalue | |
| +0x20 | macro2value | |
| +0x24 | macro3value | |
| +0x28 | macrodesc | string |

The physical key maps to its table position through a `std::map` at `this+0xAEC`, filled while parsing the XML
(0x45c6cf..0x45c7e0). The map key is `strtol(code,16)` and the value is the `key_index` attribute. The encoders
look up `map[record.keyvalue].key_index` at 0x41aeb8, 0x41b9d6 and 0x41bba7. **[code]**
(If that map is empty, the code falls back to index 0.)

Key indices from KeyboardLayout.xml (68-key layout, max index 108):
Esc 0, VOL+ (0xE9) 13, VOL- (0xEA) 14, 1..0 17..26, -_ 27, =+ 28, Tab 32, Q..P 33..42, [ 43, ] 44, Caps 48,
A..L 49..57, ;: 58, '" 59, \| 60, LShift 64, Z..M 65..71, ,< 72, .> 73, /? 74, RShift 75, Enter 76,
LCtrl 80, LWin 81, LAlt 82, Space 83, Fn (0xAF) 85, RCtrl 87, Left 88, Down 89, Up 90, Right 91,
Backspace 92, Home 104, PgUp 105, Del 106, PgDn 108.

---------------------------------------------------------------------------------------------------

## 3. The 512-byte table

```
offset = key_index * 4          (entries 0..126 usable; 127 overlaps the signature)
entry  = [b0 = class, b1, b2, b3]
0x1FE  = AA 55                  (word 0x55AA, written at 0x41b46e (wired) / 0x41ba6a (dongle))
```
The wired buffer is `ebp-0x360`, memset to 0 for 0x30C bytes at 0x41adf4, and sent with length 0x200.
The dongle buffer is `ebp-0x294`, memset to 0 for 0x240 bytes at 0x41b924. **[code]**
Keys with no DB record, and records of type 0 or 4, are left as `00 00 00 00`. **[code]**
**[inf]** `00 00 00 00` probably means "firmware default for this key". The single-key dongle path writes
`02 00 <own HID> 00` instead when no record exists (0x41bb7c..0x41bc08), which suggests the two are equivalent.

---------------------------------------------------------------------------------------------------

## 4. Entry encoding per function type

Encoders: wired inline 0x41aece..0x41b3d7; dongle/per-key 0x41b4f0 (`encode(record*, uint8_t out[4])`).
The two are identical. **[code]**
UI mapping: the type combo in the "Custom Functions" dialog is filled at 0x411ebc..0x412195 with item data = macrotype.
The labels come from the string IDs in `language/1033.lan`. **[code]**

| macrotype | UI (string id) | entry bytes | notes |
|---|---|---|---|
| 0 | "Default Function" (160), message 0x25E at 0x41e336 | `00 00 00 00` | no bytes written |
| 1 | "Disabled"/"Disable Key" (120/161), message 0x25F, record type=1 value=1 (0x5a90e0) | **`05 03 00 00`** | 0x41aed6 |
| 2 | "Key Functions" (121) | `02 <modmask> <HID> 00` | see 4.1 |
| 3 | "Macro Definition" (122) | `06 <macro#> <mode> <count>` | see 4.3 |
| 4 | (no UI entry found) | `00 00 00 00` | 0x41b0ff, nothing written |
| 5 | "Mouse Function" (124) | `01 <sub> <val> 00` | see 4.4 |
| 6 | "Multimedia" (125) | `03 <usage lo> 00 00` | see 4.5 |
| 7 | "Windows Shortcut" (126) | `02 <modmask> <HID> 00` | see 4.6 |
| 8 | "Open Program" (127) | `05 02 <id> 00` | software action, see 4.7 |
| 9 | "Open Website" (128) | `05 02 <id> 00` | " |
| 10 | "Send Text" (129) | `05 02 <id> 00` | " |
| 11 | "Switch Configuration" (130) | `05 02 <id> 00` | encoder handles it, but it is not in the type combo of this build |
| 12 | Key picker codes >= 0x1F4 (0x41e5be: `type = value>=500 ? 12 : 2`) | `03 <lo> <hi> 00` or `02 <mod> <key> 00` | see 4.2 |
| 13 | "Multiple keys" (131) | `07 <HID1> <HID2> <HID3>` | 0x41afd4; values from 0x414431..0x41455a |

### 4.1 Type 2 – plain key, key with modifier, modifier only  (0x41aee5..0x41af88)
`macrovalue` is the HID usage. `macro2value` is **one** modifier as a HID usage E0..E7 (or 0). The dialog converts Windows VKs to HID with
0x46a820 (for example VK_LCONTROL 0xA2→E0, VK_LSHIFT 0xA0→E1, VK_LMENU 0xA4→E2, VK_LWIN 0x5B→E3, RCtrl E4, RShift E5, RAlt E6, RWin E7).
0x46ae30 turns E0..E7 into the bit mask 01,02,04,08,10,20,40,80 and returns any other value unchanged.
```
b0 = 02
b1 = mask(macro2value)
b2 = macrovalue (HID)
if macro2value == 0 and macrovalue in E0..E7:  b1 = mask(macrovalue), b2 = 00   (modifier-only key)
special: macrovalue == D3        -> 02 00 D3 00
special: macrovalue in F1,F2,F3  -> 02 00 <keyvalue of THIS physical key> <F1|F2|F3>
```
**[inf]** F1/F2/F3 are probably special key-picker codes (mui.dll lists "fn1"/"fn2"). D3 is not identified.
The UI allows only one modifier per binding.

### 4.2 Type 12 – key-picker "special" codes  (0x417930 table, 0x41b357)
| macrovalue | mui.dll label | entry |
|---|---|---|
| 0x1F4 | My PC | `03 94 01 00` (Consumer 0x0194) |
| 0x1F5 | Calculator | `03 92 01 00` |
| 0x1F6 | Email | `03 8A 01 00` |
| 0x1F7 | Multimedia | `03 83 01 00` |
| 0x1F8 | Play/Pause | `03 CD 00 00` |
| 0x1F9 | Stop | `03 B7 00 00` |
| 0x1FA | Previous | `03 B6 00 00` |
| 0x1FB | Next | `03 B5 00 00` |
| 0x1FC | Vol + | `03 E9 00 00` |
| 0x1FD | Vol - | `03 EA 00 00` |
| 0x1FE | Mute | `03 E2 00 00` |
| 0x1FF | Website/Home | `03 23 02 00` (AC Home 0x0223) |
| 0x200 | Web Refresh | `03 27 02 00` |
| 0x201 | Web Stop | `03 26 02 00` |
| 0x202 | Web Backward | `03 24 02 00` |
| 0x203 | Web Forward | `03 25 02 00` |
| 0x204 | Favorites | `03 2A 02 00` |
| 0x205 | Web Search | `03 21 02 00` |
| 0x206 | Show Desktop | `02 08 07 00` (Win+D) |
| 0x207 | Screen LED switch | `02 00 9E 00` (keyboard usage 0x9E) |

Class 03 therefore means **Consumer usage, 16-bit little-endian in b1..b2**. **[code]** for the bytes; **[inf]** for the labels (taken from the order of mui.dll strings).

### 4.3 Type 3 – macro  (0x41b004..0x41b0b9; dialog 0x41412b..0x414275)
```
06 <macro#> <mode> <count>
macro#  = 0-based position of the macro whose id == macrovalue in the macro list returned by 0x402dd0
mode    = macro2value: 0 "Play once"(196), 1 "Play multiple times"(197), 2 "Stop when key again"(198)
          (combo filled at 0x41258c..0x41266d, item data 0/1/2)
count   = macro3value, only set when mode==1: clamped to 1..255 (0x41423f..0x414267), otherwise 0
```
If the macro id is not found, the entry is left as zeros. **[inf]** `macro#` should match the order in which macros are uploaded (cmd 0x20 / dongle `0B 1C`); cross-check against the macro spec.

### 4.4 Type 5 – mouse  (0x41b108..0x41b1bb; combo data at 0x4132bf..0x41350d)
| value | label | entry |
|---|---|---|
| 1 | Left-Click (170) | `01 01 01 00` |
| 2 | Middle-Click (172) | `01 01 04 00` |
| 3 | Right Click (173) | `01 01 02 00` |
| 4 | Double-Click (174) | `01 01 03 00` |
| 5 | Scroll Up (175) | `01 03 01 00` |
| 6 | Scroll down (176) | `01 03 FF 00` |
| 7 | "Mouse Button 5" (177) | `01 01 08 00` |
| 8 | "Mouse Button 4" (178) | `01 01 10 00` |

**[inf]** b1=01 means button mask in b2, b1=03 means wheel delta (signed) in b2. The vendor labels appear to swap buttons 4 and 5 relative to the HID bit order.

### 4.5 Type 6 – multimedia  (0x41b1c0..0x41b251; combo 0x4135b2..0x4137ad)
1 Play/Pause `03 CD 00 00`, 2 Stop `03 B7 00 00`, 3 Previous `03 B6 00 00`, 4 Next `03 B5 00 00`,
5 Volume+ `03 E9 00 00`, 6 Volume- `03 EA 00 00`, 7 Mute `03 E2 00 00`.

### 4.6 Type 7 – Windows shortcut  (0x41b256..0x41b331; combo 0x413889..0x413993)
1 Show Desktop `02 08 07 00` (Win+D), 2 My Computer `02 08 08 00` (Win+E), 3 Lock `02 08 0F 00` (Win+L),
4 Close Window `02 01 1A 00` (Ctrl+W), 5 Switch Windows `02 04 2B 00` (Alt+Tab).
The encoder also has values that are not in the UI combo: 6 `02 01 06 00` (Ctrl+C), 7 `02 01 19 00` (Ctrl+V), 8 `02 01 1B 00` (Ctrl+X),
10 `02 02 00 00`, 11 `02 01 00 00` (b0/b1 written as a word; meaning unknown, **[inf]** possibly intended for Sleep/Power, strings 191-194).

### 4.7 Types 8–11 – host-side actions  (0x41b3c3)
The entry is `05 02 <id&0xFF> 00`. `id` is the `macro_value` rowid that `INSERT INTO t_key_otherdata(macro_desc,param,type)` (0x403760) returns,
holding the path, URL or text. When the key is pressed, the keyboard sends an input report **`04 40 <id>`**. The app reads it in 0x448040
(`hid_read_timeout(…,0x41,10)`, test at 0x448087..0x4480af) and runs `0x41bc40(id)`, which looks up t_key_otherdata and runs the action.
A replacement app has to listen for `04 40 xx` to support these. **[code]** (The dongle path delivers the same event as window message 0x444 at 0x443b92. Its source report was not traced.)

### 4.8 Concrete examples (base layer)
| Want | key_index → offset | entry bytes |
|---|---|---|
| A sends B | A=49 → 0xC4 | `02 00 05 00` |
| key sends Ctrl+C | any | `02 01 06 00` (or type 7 value 6, same bytes) |
| key sends Volume Up | any | `03 E9 00 00` |
| key sends left click | any | `01 01 01 00` |
| macro #3 (3rd in list), play once | any | `06 02 00 00`; repeat 5x: `06 02 01 05`; toggle: `06 02 02 00` |
| disable CapsLock | 48 → 0xC0 | `05 03 00 00` |
| default | any | `00 00 00 00` |
| Mac profile: LWin acts as LAlt | 81 → 0x144 | `02 04 00 00` (built-in record, 0x407770) |
| launch program id 7 | any | `05 02 07 00` |

---------------------------------------------------------------------------------------------------

## 5. Wired command sequence  (0x41ad70, all via send65 0x467f50)
```
[1] 04 18 00 00 00 00 00 00 00 ... (64 B)          begin           0x41adbd
[2] 04 11 00 00 00 00 00 00 08 00 ... (64 B)        keymap, 8 pkts  0x41add1/0x41adda
[3] 8 x 64 B = table[0x000..0x1FF]                  send65(buf,0x200) 0x41b483
    (packet 8 ends with ... AA 55)
[4] 04 02 00 ...                                    end/apply       0x41b498
[5] 04 F0 00 ...                                    commit          0x41b4ac
```
send65 handles the reply check per packet (`payload[2]==1` OK, `0xFF` retry). During the transfer the flag `this+0x916` is set to 1 (0x41adab / 0x41b4c3).
The task dispatcher (0x4461c0) runs this for task 5 (full update) **and** task 6 (single-key change) when `this+0x940 == 0` (wired) (0x4463de..0x446418).

## 6. Dongle sequences  (dongle33 0x468790; 33-byte buffer, buf[0]=report ID 0, checksum at buf[0x20])

### 6.1 Full table (task 5, `this+0x940 == 2`) – 0x41b8e0
```
for seq in 0..18:
   buf[0]    = 00                      report ID
   buf[1]    = 10                      command               (0x41ba92)
   buf[2]    = 1C  (seq<18) / 08 (seq==18)  valid length     (0x41ba90..0x41baab)
   buf[3]    = seq                     frame index 0..0x12   (0x41bab8)
   buf[4..31]= table[seq*28 .. seq*28+27]  (28 bytes, copied every time) (0x41ba8d..0x41bab1)
   buf[32]   = sum(buf[0..31]) & 0xFF
   dongle33(buf); Sleep(2)                                    (0x41bac0, 0x41baca)
```
Payload as seen on the wire (without the report ID): `10 1C 00 t0..t27 CK`, `10 1C 01 t28..t55 CK`, … , `10 08 12 t504..t511 00x20 CK`.
18*28 + 8 = 512. In the last frame `t510 t511 = AA 55`, and the remaining 20 bytes are zero because the buffer is 0x240 bytes and zero-filled.
No begin/end/commit frames. **[code]**

### 6.2 Single key (task 6, dongle) – 0x41bb30
Runs only if `this+0x8F8` (current physical key) > 0.
```
buf[1..2] = 03 04            (dword 0x02000403 at 0x41bb88 also sets buf[4]=02)
buf[3]    = key_index        (map lookup, 0x41bba7..0x41bbb1)
buf[4..7] = entry            default 02 00 <keyvalue> 00 (0x41bb88..0x41bb96);
                             if a DB record exists (0x407250, fnlayer=0): buf[4..7]=0, then encode() (0x41bbef..0x41bc03)
buf[8..31]= 00
buf[32]   = checksum
```
Example: A→B over the dongle: `03 04 31 02 00 05 00 00 … CK`.

## 7. Fn layer

* Every path that builds data for the device hard-codes `fnlayer = 0`: 0x41ae43 (wired), 0x41b96d (dongle full), and 0x41bbdc
  (single key; bound as SQL parameter 3 at 0x4072df). **[code]**
* `fnlayer = 1` records are read only by the UI (0x41a430 / 0x41a840 use `this+0x8FC`) and by XML profile export (0x41d318). **[code]**
* Clicking the base/Fn layer tab (0x41be20 sets `+0x8FC=0`, 0x41bf90 sets `+0x8FC=1`) queues task 5. That re-sends the **base** table. **[code]**
* 0x41aa30: if a key is assigned type 2 with value 0xAF (Fn), the same record is also saved to the other layer (0x41acdd..0x41ad07). **[code]**
* Conclusion: **this app version provides no protocol for writing the Fn layer.** The Fn layer is fixed in firmware. **[inf]** A different command
  (for example `04 12`) might exist in firmware, but nothing in this binary sends one.

## 8. Read-back
No command reads the keymap. The only inbound traffic handled in this area is `04 40 <id>` (host action trigger) and `04 73 <b>` (sets
`this+0x52F48`, unrelated) in 0x448040. **[code]** (absence: every send65/dongle33 call site was checked, and none issues a read-type keymap command)

## 9. Reset / restore factory
* "Reset Keyboard Data" (string 410, button control id 0x10A, dispatch 0x442832 → **0x442d10**): after a confirm dialog it runs
  `DELETE FROM t_key_data WHERE profile = ?` (0x407430), reloads the UI, and queues task 5. On the wire this is a normal keymap upload
  with an all-zero table (+AA 55). **[code]**
* "Restore Factory Settings" (string 411, id 0x10B → **0x442e80**): resets the local DB/config (0x401750, 0x40ceb0, …), then queues
  task 3 (0x4178a0, profile/lighting), task 5 (keymap), and task 8 (per-key RGB, 0x43b660 / 0x435f40) at 0x4432fa / 0x443339 / 0x443381. **[code]**
  No dedicated device reset opcode was found. **[inf]**

## 10. Open questions
* Meaning of D3 and F1/F2/F3 in type 2, and of type 7 values 10/11.
* Whether firmware treats `00 00 00 00` and `02 00 <own> 00` the same (very likely).
* Exact macro list ordering used for `macro#` (depends on 0x402dd0's SQL); verify with the macro spec.
