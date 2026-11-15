`hfd_python.json` is every byte the original Python prototype of Halcyon wrote to the keyboard for a fixed set
of commands, on both the 2.4 GHz dongle and the USB cable. That prototype is what was first confirmed on real
hardware. The test `protocol::hfd::parity::wire_bytes_match_python` replays the same commands through the Rust
transports and requires identical output, so behaviour confirmed on hardware carries over.

Change this file only together with a protocol change that has been confirmed on hardware.
