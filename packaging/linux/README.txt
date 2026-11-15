Halcyon for Linux

Needs WebKitGTK 4.1 (most current desktops have it; on Debian/Ubuntu: sudo apt install libwebkit2gtk-4.1-0).

1. Let your user account talk to the keyboard (once):
     sudo cp 70-halcyon.rules /etc/udev/rules.d/
     sudo udevadm control --reload && sudo udevadm trigger
   then unplug and replug the keyboard or its dongle.
2. Run:  ./halcyon        (or ./halcyon --demo to look around without a keyboard)

Use at your own risk: read DISCLAIMER.md first.
