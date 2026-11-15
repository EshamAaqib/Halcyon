# LEOBOG AMG65: LED matrix display, LCD (TFT) screen, time/stats push

Static analysis of DeviceDriver.exe v1.0.3.1 (image base 0x400000) and mui.dll.
Tags: **[code]** = read directly from disassembly; **[inf]** = inferred / not proven.
"Wired framing" = 64-byte payload with report ID 0 prepended (65-byte hid_write), as in PROTOCOL.md.

---------------------------------------------------------------------------------------------------
## 1. The two displays

| | LED matrix ("矩阵灯带屏", LED tab) | LCD ("TFT", 1.14" panel) |
|---|---|---|
| Size | **63 x 5 = 315 RGB LEDs** [code] `mov [edi+0x52b24],0x13b` @0x41698b | **240 x 135 px** [code] `[+0x52f14]=0xF0`, `[+0x52f18]=0x87` @0x416a23/0x416a2d |
| Pixel format | 3 bytes R,G,B [code] 0x43476b..0x43477c | RGB565, little-endian [code] mui.dll `LCDViewList::GetImageRGB565Data` @0x10050ae4 |
| Stored animation | cmd 0x31 (boot logo, <=200 frames), cmd 0x33 (active layer, <=260 frames) | cmd 0x72, <=130 frames per slot |
| Live streaming | cmd 0x35, one frame per transaction | none found |
| Transport for bulk | big4k (4096-byte reports, 2nd interface 0xFF67/0x61) | big4k |

The firmware file name `_1.14LCD_HFD80CP100_V1.31_20250215_0x01DD.exe` (string @file 0x19eaec) confirms
a 1.14" LCD. 1.14" panels are 240x135, which matches the geometry above.

---------------------------------------------------------------------------------------------------
## 2. LED matrix geometry and LED order

The app's logical index is `k = row*63 + col` (row 0..4 top to bottom, col 0..62). This is proven by the library
data (section 7): rendering layer 1 as 63x5 row-major shows the "LEOBOG" logo.

On the device, index order goes **panel by panel**: four 14x5 panels, then one 7x5 panel. Each panel is row-major.
The lookup table `devIndex[k]` (315 dwords @0x59c7f0) [code] matches exactly (checked for all 315 entries):

```
def dev_index(k):            # k = row*63 + col
    row, col = divmod(k, 63)
    if col < 56: return (col // 14) * 70 + row * 14 + (col % 14)
    return 280 + row * 7 + (col - 56)
```
This is a separate strip/matrix of its own. It is not mapped onto the keys: the per-key light_index from
KeyboardLayout.xml is not used here [inf].

---------------------------------------------------------------------------------------------------
## 3. Matrix: stored animation upload (wired) — cmd 0x31 / 0x33

Functions: **0x434410** (cmd 0x31, layer id 1 = "Boot Logo", max 200 frames) and **0x434970** (cmd 0x33, layer
id `[this+0x900]` = currently applied layer, max 260 = 0x104 frames). The two bodies are identical apart from these values [code].
They run from the task queue at 0x4464c1 (task 10) and 0x4464e6 (task 11).

### 3.1 Payload buffer [code 0x4344dd-0x4347b0]
```
off 0..1   frameCount  (u16 LE)   = min(totalPixels/315, 200|260)
off 2..3   speed       (u16 LE)   = 0x66 - layer.speed   (default 0x50 if layer row missing)
off 4..    frames: frameCount * 315 * 3 bytes; pixel (frame f, logical k) at 4 + 3*(f*315 + devIndex[k])
           bytes R,G,B (scaled by brightness, see below)
off 4+3N+1, 4+3N+2  = AA 55     (N = number of pixels written; note the one-byte gap at 4+3N) [code 0x4347b0]
```
* `layer.speed` = column `speed` of t_ledlayer_data (struct +0x10), and `layer.brightness` = column `brightness`
  (struct +0x14) [code 0x40d950: column 4 -> +0x10, column 5 -> +0x14].
* Brightness: `a = brightness*255/100` (default a=100 if the DB read fails). Each channel is `c*a/255`
  (the float blend term is `(1-a/255)*0.0`, so it is zero) [code 0x4345e9, 0x4346aa-0x434767; constants
  0x5a9030=1.0, 0x5a90c0=255.0, 0x5a9004=0.0].
* Source colours are Windows COLORREF `0x00BBGGRR` (DB `rgb_value`). The low byte is R [code 0x4346a1-0x4346d3].
* The upload has one global speed only. The per-frame `delay_time` (t_ledframe_data) is not sent [code].
  The speed unit is unknown; a higher `speed` value gives a smaller number [inf].
* blocks = ceil((frameCount*945 + 4) / 4096) [code 0x4344f2-0x43451c].

### 3.2 Wire sequence [code]
```
send65:  04 18 00 ...                                   (begin)
send65:  04 31|33 00 00 00 00 00 00 <blocks lo> <blocks hi> 00 ...   (header; bytes 8-9 = number of 4 KB blocks)
big4k x blocks:  report ID 0 + 4096 bytes of the buffer, sequential, no offset field   (0x434802)
send65:  04 02 00 ...                                   (end)
send65:  04 F0 00 ...                                   (commit)
```
big4k (0x468650) [code]: it drains the 0xFF67 interface, writes 0x1001 bytes, sleeps 5 ms, then polls the
control interface (65-byte reads, 600 ms timeout, up to 100 tries) until reply payload[2]==0x01. The tail of the
last block is uninitialised heap memory in the vendor app; send zeros.

---------------------------------------------------------------------------------------------------
## 4. Matrix: live frame streaming (wired) — cmd 0x35

Functions 0x434ed0, 0x435180 and 0x4352d0 [code]. They are called from the timer path 0x446701 -> 0x433160 while the
LED page is active:
* 0x4352d0: sedentary-reminder animation (`LEDLayerList::GetSitRemindLEDEffect`). Runs when `[+0x960]` is set
  (section 6.3).
* 0x435180: applied layer preview (`GetCurApplyLEDEffect`), when `[+0x958]` is set (0x43548f). Header byte 8 is 0x10.
* 0x434ed0: (a) while the LED editor is open, sends the frame being edited (`LEDEffect::GetLEDEffect`);
  (b) otherwise, if the applied layer id is 2..7, sends `GetCurApplyLEDEffect`. Layers 2..7 are CPU Usage,
  GPU Usage, Internet Speed, Datetime, Sedentary Reminder and Musical (string ids 861-866). **These widgets are
  rendered on the PC by mui.dll and streamed. The keyboard does not render them** [code for the id range
  check @0x435015/0x4351a2; rendering inside mui.dll is inf].

Packet [code 0x434f35-0x435168]:
```
send65:      04 18 ...
send65:      04 35 00 00 00 00 00 00 0F ...      (byte 8 = ceil(945/64) = 15; 0x435180 uses 0x10)
send65slow:  buffer of 945+128 = 1073 bytes -> sent as 1073>>6 = 16 64-byte packets, each waits for a reply
             buf[3*devIndex[k] + 0..2] = R,G,B for logical pixel k   (0x434fd0 / 0x4350f0)
             buf[946..947] = AA 55
send65:      04 02 ...                           (no F0 commit)
```
Note: 0x435180 and 0x4352d0 index the other way round: `out[3*k] = src[devIndex[k]]` (0x435250). Their source buffer
appears to be in device order already [inf]. For an open-source implementation, build the frame in device order
with devIndex as in 0x434ed0. No pixel scaling is applied to streamed frames.

How the device switches between the stored animation and the stream is not visible. Receiving 0x35 probably
overrides playback [inf].

### Music visualizer
`led_music_visualizer/amplitude/backmode/backmode_bright` (0x432ff2..0x43307f) are only loaded into UI controls.
The app takes the audio spectrum (`[+0x50f20]`) and maps it through the tables at 0x5d3880 and 0x59cce4. It then
calls `LEDLayerList::SetMusicLEDEffect` (0x43333a) or, for layer 7, `LEDEffect::SetCurrentLEDEffect`. The resulting
frame goes out through cmd 0x35 like the other widgets [code for the calls; spectrum source inf]. No dedicated
music protocol command was found.

---------------------------------------------------------------------------------------------------
## 5. LCD image / GIF upload (wired) — cmd 0x72

Function **0x42a1d0** (task 17, 0x4465ed) [code]. Parameters are set in the device init at 0x4169d3-0x416a2d:
header 0x100 bytes, W=240, H=135, max frames 130 (0x82, the same for all 5 `LCDViewCtrl` slots, 0x52efc..0x52f0c).

### 5.1 Buffer
```
size   = 0x100 + frames*240*135*2   (64 800 bytes per frame), buffer pre-filled with 0xFF (0x42a2ad)
off 0          frameCount (u8, <= 130)                                  (0x42a417)
off 1+i        delay of frame i = max(1, delay_ms/2) as u8  (unit 2 ms) (0x42a45f-0x42a480)
off 0x100 + i*64800   frame i: 240x135 RGB565, 2 bytes per px, low byte first
```
RGB565 conversion (mui.dll 0x10050ae4) [code]: `v = (R&0xF8)<<8 | (G&0xFC)<<3 | B>>3`, stored `v&0xFF, v>>8`.
Each frame PNG is scaled to 240x135 by the app (GetImageRGB565Data(path, dst, 240, 135)). Rows are
written top to bottom, left to right [inf: depends on the DIB orientation in mui].
blocks = ceil(size/4096) (0x42a25a-0x42a27d).

### 5.2 Wire sequence [code 0x42a907-0x42aa88]
```
send65:  04 18 ...
send65:  04 72 <slot> 00 00 00 00 00 <blocks lo> <blocks hi> ...   slot = LCDViewList::GetCurSel()+1 (also kept in [+0x52f2c])
big4k x blocks: sequential 4 KB chunks; Sleep(500 ms) after the FIRST block only
send65:  04 02 ...                                                  (no F0 seen)
```
Frame sources: t_lcdframe_data (delay_time, file_name). GIFs are split into PNGs by `LCDViewList::LoadGIFImageFile`,
which runs on the host only. `gif\LEOBOG AMG65\0.gif` (128x128) is a stock animation that gets scaled. Defaults:
`%s\default_led\<model>\<n>.png` (0x42a565). The meaning of the slot (which of the 5 LCD slots) is
[inf]. The code uses LCDViewList's selection, while the frame limit comes from LCDViewCtrl's selection.

### 5.3 Selecting what the LCD shows
The only on-device selector found is byte 1 of the time/stats data block (section 6), which is the same
`LCDViewList::GetCurSel()+1`. So the device shows its stored slot `n`. The built-in clock/stat screens are
presumably firmware screens that use the pushed values [inf].

---------------------------------------------------------------------------------------------------
## 6. Time sync + PC stats + weather — cmd 0x28 (wired) / 0x0C (dongle)

PROTOCOL.md's "0x28 settings block, data[2]=0x5A" is this packet.

### 6.1 Wired [code 0x42ab10 (time only), 0x447450 (time + stats)]
```
send65: 04 18 ...
send65: 04 28 00 00 00 00 00 00 01 ...       (1 data packet)
send65: data (64 bytes):
  [0]=00  [1]=slot (LCDViewList sel+1)  [2]=5A
  [3]=year%2000 [4]=month [5]=day [6]=hour [7]=minute [8]=second [9]=00 [10]=dayOfWeek (0=Sunday, SYSTEMTIME)
  [11..12]=00
  --- only in 0x447450: ---
  [13]=CPU load %   [14]=CPU temp °C   [15]=GPU load %   [16]=GPU temp °C      (each = _wtoi of the data.ini value, low byte)
  [17]=weather temp [18]=dayTemp [19]=nightTemp [20]=weather code
  [62]=AA [63]=55
send65: 04 02 ...      (no F0)
```
The time comes from GetLocalTime. The stats come from `HardwareMonitor\data.ini` [Hardware] CPU / CPU_Temperature / GPU /
GPU_Temperature. That file is written by OpenHardwareMonitorServer.exe (LibreHardwareMonitor). `Network_Upload/Download` are in
the ini but **are not sent**, so the "Internet Speed" widget is matrix-rendered (section 4) [code: no reference].

Weather (0x4474dd-0x447c01): GET `https://api.ip138.com/weather/?&type=1&token=…`, JSON keys temp/dayTemp/nightTemp/weather.
The code is the first substring match in this order: 晴→0, 云→1, 小雨→3, 中雨→4, 大雨→5, 阵雨→6, 暴雨→7, 大暴雨→8,
雷阵雨→9, 雪→0x12 (18). Codes 8 and 9 cannot be reached because of the order. If nothing matches, the last code is kept.
Weather is refreshed when more than 18 s (0x4650 ms) have passed or on a forced call.

Scheduling: `0x447450(0)` runs on the timer (0x446712). It sends at most once per 1000 ms, and only when `[+0x52f48]!=0`.
That flag is set by an unsolicited input report `04 73 xx` (xx!=0 means enabled) parsed at 0x448087 [code].
The device therefore requests the stats stream. `0x447450(1)` forces a send after the time sync at 0x446ff5.

### 6.2 Dongle time sync [code 0x42ac40]
dongle33 payload (32 bytes after report ID 0, checksum at [0x20]):
```
0C 10 00 | 00 slot 5A YY MM DD hh mm ss 00 DOW 00 00 00 AA 55 ...
```
This is the same data block shifted by 3, as a 16-byte block with AA55 at block offset 14-15. The stats fields (13-20)
do not fit, and the dongle variant does not send them. After it, `0x447450(1)` is still called (0x447005), but that
function only uses wired send65 [code]. So stats over 2.4 GHz are probably not delivered [inf].

### 6.3 Sedentary reminder [code 0x434290]
`led_sit_remind` sets the interval: 1=30 min, 2=45 min, 3=60 min, 4=1 min (0x1b7740/0x2932e0/0x36ee80/0xea60 ms),
0=off. The check uses GetLastInputInfo. When it fires, `[+0x960]=1` for 6 s (0x1770) and the reminder
animation streams over cmd 0x35 (0x4352d0). It is host-driven and has no device command.

---------------------------------------------------------------------------------------------------
## 7. ledlayer_v2.data format (LED library)

This is a plain **SQLite 3** database (not encrypted) [read with sqlite3]:
```
t_ledlayer_data(led_id PK, profile, name, desc, speed, brightness, type, state)
t_default_led_frames(frames_id PK, layer_id, frameIndex, led_index, rgb_value)   -- 496 230 rows
t_led_frames(frames_id PK, layer_id, frameIndex, led_index, rgb_value)           -- 72 475 rows (user layers)
```
* `rgb_value` = COLORREF 0x00BBGGRR. `led_index` = logical k (row*63+col); this is not the device order.
* t_ledlayer_data rows: 1 开机logo, 2 CPU使用率, 3 GPU使用率, 4 网速, 5 时间显示, 6 音乐律动, 7 音谱,
  8 俄罗斯方块, 9 下雨1, 10 多彩纵横, 11 下雨2 (speed=10, brightness=10, type=2).
* t_default_led_frames: layer 1 = 166 frames x **315** (63x5 boot logo, verified visually). Layers 2-6 =
  250/133/350/113/60 frames x **490** px. They render correctly as **70x7**, a larger matrix from another product.
* t_led_frames in the shipped file has 325 px per frame (65x5?). This is stale data from another variant [inf].
* The app reads rows in table order with `SELECT * … WHERE layer_id=?` (0x40d122, cap 152 390 rows).
  It flattens them and uses frames = rows/315 (0x430717/0x43078e). There is no resampling, so only 315-px layers
  map correctly onto the AMG65 [code]. First-run defaults (0x433360 -> 0x434190(libId, userId)): lib 1 -> Boot
  Logo (id 1); ids 2-7 are live widgets with no frames; lib 2 -> id 8 Christmas, 3 -> 9 Aurora, 4 -> 10 Game, 5 -> 11 Rainbow, ...
* To convert a library layer to a 0x31/0x33 upload: for each frame, for k in 0..314, put RGB(rgb_value) at
  `4 + 3*(f*315 + dev_index(k))`.

LED GIF import/export (`LEDControl::LoadLedGifFile/SaveLedGifFile`, tasks 13/14) and "LEDExport\LED-%d.json"
are host-side file formats only. They have no protocol impact.

---------------------------------------------------------------------------------------------------
## 8. Dongle coverage
No dongle33 or 0xFF59 path exists for cmds 0x31/0x33/0x35/0x72 or for the stats packet. The value 0xFF59 is not referenced at all,
while 0xFF67 (0x468ec1) and 0xFF68 are. Matrix and LCD content therefore appear to be **wired-only** in this app
version [inf from absence]. Only the time sync has a dongle form (`0C 10`).

## 9. Not display-related (checked)
0x43dad0/0x43e0d0 (04 19 / 04 15, 30-byte records; dongle `09 1C`) belong to the macro list (HFDListBox). 0x43ba50 (04 F5,
8 packets) runs on timer page 4 (probably per-key lighting) [inf]. 0x448170 (`20 01` dongle) looks like a link/status poll and is unrelated to the displays [inf].
