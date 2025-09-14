# wireless-display

Use your laptop as a second monitor for your Windows desktop PC over WiFi.



## Usage

You need a virtual display driver on the host machine to make this work. On Windows, you can use [Virtual-Display-Driver](https://github.com/VirtualDrivers/Virtual-Display-Driver). I don't know a solution for other operating systems yet.

**Important:** Make sure both machines have ffmpeg installed and it's in your PATH. Also set `FFMPEG_DIR` environment variable to the directory containing ffmpeg include files.

Install the program from cargo:
```
cargo install wireless-display
```

On host machine (PC you are currently using):
```
wireless-display server
```

Then on client machine (PC you want to use as second monitor):
```
wireless-display client
```

See `wireless-display server --help` and `wireless-display client --help` for more options.



## Why create this?

My desk can only fit one desktop monitor, but I constantly need more screen space for development. I have a laptop so I thought why not use it as a second screen. I found some other solutions online, but they are either paid, less configurable, or overly complicated. So I decided to build my own.



## Useful links

[ffmpeg build instructions](https://github.com/zmwangx/rust-ffmpeg/wiki/Notes-on-building)
