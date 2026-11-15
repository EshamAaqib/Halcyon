# Per-key RGB, keyboard settings, reset: AMG65 / DeviceDriver.exe (static analysis)

Legend: **[code]** = read directly from disassembly. **[inf]** = inferred (naming, UI pairing, purpose).
Offsets: "wired data[i]" = byte i of a 64-byte payload (report ID 0 is prepended by send65 0x467f50).
"dongle p[i]" = byte i of the 32-byte payload (dongle33 0x468790 buf[1+i]). The checksum goes in p[31].

## 0. Transport details that matter here
- send65 0x467f50 [code]: if len<=0x41, sends one 64-byte payload and reads a reply (200 ms timeout). If reply[2]==0xFF, it resends once. Success = reply[2]==0x01. If len>0x41, it sends len>>6 chunks of 64 bytes (buf += 0x40 each time) and reads after each one. So `send65(buf,0x200)` sends 8 packets = bytes 0..0x1FF.
- send65slow 0x468170 [code]: the same, plus Sleep(6) between writes and a 500 ms read timeout.
- dongle33 0x468790 [code]: the checksum is the 8-bit sum of buf[0..0x40] (report ID 0 + payload), written to buf[0x20]. It writes 0x21 bytes, waits Sleep(5), then reads up to 20 times (5 ms timeout, 10 ms gap) until reply[0..2] == sent p[0..2]. The reply is copied back into buf[0..0x20], so the reply payload starts at buf[0].
- this+0x940 holds the connection type: 0 = wired, 2 = dongle. In the dispatcher at 0x446380.., task 8 picks wired 0x43b660 or dongle 0x435f40 [code].

## 1. Per-key colour record format
Each key is **4 bytes: `{light_index, R, G, B}`** [code].
- In 0x43b660 and 0x435f40, each entry is written at **offset light_index*4**, so the table is sparse and indexed by LED. Byte 0 is again the low byte of light_index. Bytes 1..3 are bytes 0..2 of the DB `rgbvalue` int (record+0xC..0xE). That int is a Windows COLORREF 0x00BBGGRR, so byte1=R, byte2=G, byte3=B. [code: the read-back at 0x43bb7e rebuilds `B<<16|G<<8|R` from bytes 3,2,1.]
- light_index comes from the map at this+0xAF4 (keycode -> light_index). The map is filled from the `light_index` attribute in KeyboardLayout.xml (0x45c77b..0x45c7f4) [code]. In the DB record, keycode is at +8. The table is t_key_rgb_data(rgb_id, light_id, keycode, rgbvalue), and the loader 0x40a480 copies the columns to +0,+4,+8,+0xC.
- The fallback table at 0x46a090 is a HID-code-to-index jump table (Esc=1). It does not match the XML (Esc=0) and looks like a leftover from another model. Use the XML.
- In the XML, light_index runs 0..108 across 69 keys. Every key fits in the 0x200-byte (128-entry) wired table.
- Unused entries are all zero (memset) [code].

### 1a. Persistent per-key ("User/Custom Light", mode 0): wired cmd 0x23 (0x43b660) [code]
```
04 18 00..                        begin (send65; the reply is checked)
04 23 00 00 00 00 00 00 08 00..   header, [8]=8 packets
<512-byte table>                  send65(buf,0x200) -> 8 x 64 B; entry i at offset i*4 = {i,R,G,B}
04 02 00..                        end
04 F0 00..                        commit
-- then 0x43b900 (switches the keyboard to per-key mode):
04 18
04 13 00 00 00 00 00 00 01        lighting cmd, 1 packet
data: [0]=0x80 [1..3]=R,G,B [8]=colortype [9]=brightness [10]=0x0F [11]=0x0F [14..15]=AA 55
04 02
04 F0
```
- The code writes AA 55 at table offset 0x23E. That is outside the 0x200 bytes it sends, so the signature never goes out on wired. Do not rely on one [code].
- In 0x43b900, R,G,B, colortype and brightness come from the DB lighting record loaded with mode argument 0x14 (`0x408830(profile,0x14,&rec)`). The record layout is the same one 0x43b3f0 uses: rec+0xD colortype, +0xE..0x10 RGB, +0x11 direction, +0x12 brightness, +0x13 speed [code].
- **So "per-key custom mode" = lighting block with mode byte 0x80 and speed and direction set to 0x0F. Brightness (0..5) still applies.** [code. That 0x80 means "custom" is inf.]
- For comparison, the normal effect block (0x43b3f0) [code]: [0]=mode [1..3]=RGB [8]=colortype [9]=brightness [10]=speed [11]=direction [14..15]=AA55. This matches PROTOCOL.md.

### 1b. Persistent per-key, dongle (0x435f40) [code]
Frame 1 selects custom mode. It is the wired lighting block shifted by 3 bytes:
```
p: 05 10 00 80 00 00 00 00 00 00 00 00 05 00 00 00 00 AA 55 00 .. [31]=cksum
   (p[3]=mode 0x80, p[4..6]=RGB=0, p[11]=colortype 0, p[12]=brightness 5, p[13]=speed 0, p[14]=dir 0)
```
The table is built exactly as on wired (entry at light_index*4 = {idx,R,G,B}). The buffer is 0x30C bytes, zeroed, with **AA 55 at offsets 0x23E/0x23F**. It goes out as **21 frames**, k = 0..20, with Sleep(2) between frames:
```
p[0]=0x14  p[1]=len (0x1C = 28; 0x10 = 16 when k==20)  p[2]=k
p[3..30] = table[k*28 .. k*28+27]   (always 28 bytes copied; only 16 are meaningful in the last frame)
p[31]=checksum
```
Total = 20*28 + 16 = 576 = 0x240 bytes = 144 entries. The last two bytes (0x23E-0x23F) are AA 55. The dongle path has no begin/end/commit frames.
Note: on the dongle, brightness is hard-coded to 5 rather than read from the DB.

### 1c. Live / streaming per-key frame: wired cmd 0x20 (0x435d10 and 0x444180) [code]
```
04 20 00 00 00 00 00 00 08        header [8]=8 (aborts if the reply is not OK)
<512 bytes> via send65slow(buf,0x200) -> 8 x 64 B
04 02
```
- This path has no `04 18` begin and no `04 F0` commit. The periodic timer drives it (0x4466ad: page 6 -> 0x435d10, page 7 -> 0x444180).
- [inf] It is the volatile real-time frame and is not stored in flash. Its uses are the animated preview in the "custom light mode" editor and music/sound-reactive lighting. So 0x20 is per-key RGB, but the *live* channel. **Use 0x23 to set persistent per-key colours.**
- Entries are **packed sequentially** in UI key-item order, not placed at light_index*4. Each entry is still `{light_index,R,G,B}`, so the firmware evidently addresses keys by byte 0 [code: edi += 4 per item].
  - 0x435d10 (CustomLightMode items, list at this+0x5266C): RGB from item+0x24..0x26. If item+0x34 == 7, each channel is scaled by `c*item[0x3C]/256` (a brightness or breathing level) [code].
  - 0x444180 (music view items, list at this+0x52870): RGB from item+0x2C..0x2E [code].
- 0x435d10 returns immediately when on the dongle. **On the dongle, 0x444180 does not send per-key data. It sends one `0B 1C` frame carrying a 16-band spectrum** [code]:
  ```
  p: 0B 1C 00 [3]=sel(ctrl 0x52884) [4]=sel(ctrl 0x5289c) [5]=ctrl0x528a4[+0x144]+[+0x148]
     [6..21]=16 bar levels (0..6) [22..26]=5 more level bytes (normally 0) ... [31]=cksum
  ```
  The bar levels come from an FFT table (this+0x50F20 / 0x20F20, table 0x59d1d0) [code]. [inf] This is the dongle's "music rhythm" mode: the firmware draws the bars itself, while the wired path streams the colours. **So the dongle has no streaming per-key path. Per-key over the dongle = §1b only.** `0B 1C` is not a per-key upload.

### 1d. Per-key read-back: wired cmd 0xF5 (0x43ba50) [code]
- Command: `04 F5 00 00 00 00 00 00 08`, sent via bulk8 0x468390. bulk8 sends the header, then for each of 8 packets writes a poll packet and reads 64 bytes, accumulating at buf+0x40*i (360 ms timeout). These bulk8 internals are at [inf] level.
- Result: same layout as the write. The entry for light_index is at offset idx*4 = {idx,R,G,B}. The app rebuilds COLORREF = B<<16|G<<8|R. `04 02` follows.
- It runs on page 4 (the per-key editor) and is wired only (it returns when this+0x940==2). Nothing reads per-key colours back over the dongle.

## 2. Keyboard settings
### 2a. Wired: cmd 0x17 (0x4454d0) [code]
```
04 18
04 17 <profile> 00 00 00 00 00 01      [2]=this+0x8F0 = profile id (set to 1 at 0x4167bd and never written as a dword elsewhere); [8]=1
data (64 B):
 [0] 0x00
 [1] game mode        1 = on, 0 = off. When game mode is off, [1..4] are all 0
 [2] disable Alt+Tab  checkbox 0x529E8
 [3] disable Alt+F4   checkbox 0x529EC
 [4] disable Win key  checkbox 0x529F0
 [5] Fn switch        KeyLayerButton 0x52558 checked = 1 (DB key "fn_switchfunction")
 [6] light sleep time _wtoi(DB "sleep_light"), default "0"
 [7] LED brightness   _wtoi(DB "led_brightness"), default "7"
 [8] 0                key response time is NOT sent on wired by this function
 [62..63] AA 55
04 02                no F0 commit
```
- Checkbox-to-name mapping: the loader at 0x445298..0x445429 calls SetCheck(gamemode=="1") on 0x529E4, disable_alttab on 0x529E8, disable_altf4 on 0x529EC and disable_win on 0x529F0 [code].
- The UI disables those three sub-options unless game mode is on (0x44542b..0x445477) [code].
- Fn switch: if fn_switchfunction=="1", 0x52558 is checked, otherwise 0x52554 (0x445100) [code].
- The checkbox handler 0x445180.. sends 0x17 immediately when this+0x91C is set.

### 2b. Dongle: `07 10` (0x4456e0) [code]
```
p: 07 10 00 00 01 [5]=alt_tab [6]=alt_f4 [7]=win [8]=fn_switch [9]=sleep_light [10]=0 [11]=key_respondtime
   .. [17..18]=AA 55 .. [31]=cksum
```
This is the same block as wired data[], placed at p[3+i], with four differences:
- p[4] (game mode) is hard-coded to 1.
- p[8] comes from DB key **"fn_switch"**, while wired uses "fn_switchfunction".
- p[10] (brightness) is 0.
- p[11] = `key_respondtime` (default "0").

### 2c. Values
- **Light sleep time** (data[6] / p[9]) [code + inf]: **0 = never, 1 = 1 min, 2 = 5 min, 3 = 30 min.**
  - [code] DB value "0" selects radio 0x52614, "1" selects 0x52618, "2" selects 0x5261C, "3" selects 0x52620 (0x4451bc..0x44527c).
  - [code] The labels 0x52624, 0x52628, 0x5262C and 0x52630 get titles str 271 "never sleep", 272 "1 minute", 273 "5 minutes" and 275 "30 minutes" (0x42299f..0x422aca).
  - [inf] Each radio is paired with the label laid out next to it (layout code at 0x460801..).
- **Key response time / debounce**: sent only in dongle p[11], as raw `_wtoi(key_respondtime)` with default 0. No UI in this app writes it, so its units are unknown [code/inf].
- **Report rate**: not present for the keyboard. "USB Report Rate" (str 812) belongs to the mouse strings, and no keyboard command uses it [inf].
- Boolean fields use 1 = enabled (key disabled, or game mode on).

### 2d. LCD clock / time sync: wired cmd 0x28, dongle `0C 10` (0x42ab10 / 0x42ac40). This is NOT a reset [code]
The 0x5A byte marks a clock packet. It is not a reset magic. The values come from GetLocalTime:
```
wired data: [0]=0 [1]=sel(ctrl 0x52a30)+1 [2]=0x5A [3]=year%2000 [4]=month [5]=day [6]=hour [7]=min [8]=sec [9]=0 [10]=dayOfWeek [62..63]=AA55
dongle p:   0C 10 00 00 [4]=sel+1 [5]=5A [6]=yy [7]=mm [8]=dd [9]=hh [10]=mi [11]=ss [12]=0 [13]=dow [17..18]=AA55
```
- Dispatcher tasks 0x14/0x15 (0x446fe0) send the time sync and then call 0x447450(1).
- 0x446e80 (task 0x10, write-only transport) sends two transactions:
  1. `04 28 [8]=1` with data [0]=0x5A [1]=sel(ctrl 0x52a28)+1, then Sleep 80, then `04 02`.
  2. `04 17 [8]=1` with data [6]=0x02 [7]=0x07 [8]=0x02 and AA55, then Sleep 80, then `04 02`.
  - [inf] This is an LCD/screen-mode select followed by a defaults push: sleep = 5 min, brightness = 7, data[8] = 2 (possibly a key response time default).
- 0x447450 builds large `04 28` packets carrying CPU/GPU hardware-monitor data for the LCD. This is outside the scope of this spec.

## 3. Reset / factory restore [code]
**No dedicated reset/factory HID command exists.** Neither 0x5A nor any other opcode does this. Both actions happen on the host: they rewrite the DB and then re-upload. The full list of wired opcodes used (0x11 13 15 17 18 19 20 23 28 31 33 35 72 F0 F5 02) has no candidate.
- "Reset keyboard" (str 80/81, 0x442d10):
  1. Shows a confirm dialog.
  2. Calls 0x407430(profile) and 0x41a430(profile) to reset/reload the DB.
  3. Queues task 5, the keymap upload (wired 0x41ad70 `04 11` 8 packets; dongle 0x41b8e0).
  4. Sleep(500).
- "Restore factory settings" (str 82/83, 0x442e80):
  1. Rebuilds the DB and UI state.
  2. Sets fn_switchfunction back to "1".
  3. Re-reads cur_customlightmode into this+0x910, the light_id used for t_key_rgb_data.
  4. Queues tasks **3** (lighting, 0x4178a0), **5** (keymap) and **8** (per-key RGB, §1a/§1b).
- A replacement tool should implement factory reset by uploading defaults: keymap, lighting, an all-default per-key table, and the settings block (0x17 / 07 10).

## 4. Read-backs, status, device info
- Per-key read: §1d (`04 F5`, wired only).
- Dongle link poll, every 15 s (0x448170) [code]: send `p = 20 01 00 ..`. Reply p[0]==0x20 and p[3]>0 means the keyboard is linked (this+0x944=1). A failed transfer sets it to 0.
- Dongle probe at enumeration (0x468940, called at 0x468fd7) [code]: send `p = 02 00 00`.
  - Success only requires reply p[0]==2, p[1]==0 and p[2]==0. The result is stored as the "online" flag (+0x228).
  - **The app never parses the rest of this reply**, so this binary cannot confirm what the bytes `40 30 00 00 45 0c 0a 80 33 01 ...` mean.
  - Guesses from the values alone [inf]: 0x3040 = firmware version, `45 0c` = VID 0x0C45 (the wired Sonix VID), `0a 80` = PID 0x800A, 0x33 = 51 % battery, 01 = linked.
  - No battery parser was found.
- Unsolicited input reports (0x448040) [code]:
  - Wired `04 73 xx`: sets flag this+0x52F48 when xx!=0.
  - Wired `04 40 nn`: the host runs custom key action nn (open program / web / text, 0x41bc40).
  - Dongle `05 A6 FF 01`: keyboard connected.
  - Dongle `05 A6 FE 02`: keyboard disconnected.

## 5. Open questions
- Does the firmware need AA 55 at 0x1FE for wired 0x23? The app never sends it, so probably not.
- What exactly mode byte 0x80 (vs mode 0) and speed/dir 0x0F mean in the per-key activation block.
- The units of key_respondtime, and the meaning of data[8]=2 in 0x446e80.
