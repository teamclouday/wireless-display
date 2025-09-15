# wireless-display

Use your laptop as a second monitor for your Windows desktop PC over WiFi.



## Usage

You need a virtual display driver on the host machine to make this work. On Windows, you can use [Virtual-Display-Driver](https://github.com/VirtualDrivers/Virtual-Display-Driver). I don't know a solution for other operating systems yet.

**Important:** Make sure both machines have ffmpeg installed and it's in your PATH. Also set `FFMPEG_DIR` environment variable to the directory containing ffmpeg include files.

Install the program from cargo:
```
cargo install wireless-display
```

On the PC you are currently using, start server with hardware acceleration enabled:
```
wireless-display server --hwaccel
```

On the PC you want to use as second monitor, start client with hardware acceleration enabled:
```
wireless-display client --hwaccel
```

See `wireless-display server --help` and `wireless-display client --help` for more options.

Make sure both machines are on the same network.

## Why create this?

My desk can only fit one desktop monitor, but I always need more screen space for development. I have a laptop so I thought why not use it as a second screen. I found some other solutions online, but they are either paid, less configurable, or overly complicated. So I decided to build my own.

WebRTC in combination with ffmpeg seemed to be good solution for this. But it's very complicated to set up. I had to use a little of help from AI to get it working. The result is still not perfect, but it's good enough for my use case.


## Useful links

[ffmpeg build instructions](https://github.com/zmwangx/rust-ffmpeg/wiki/Notes-on-building)
