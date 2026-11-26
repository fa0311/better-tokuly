# Clips for the demo's shorts

Three seconds each, 360 × 640, silent: abstract colour gradients from ffmpeg's
test sources. They show nobody and nothing. Made with:

```sh
make() {  # make <n> <lavfi source>
  ffmpeg -y -f lavfi -i "$2" -f lavfi -i "anullsrc=r=44100:cl=mono" -t 3 -r 24 \
    -c:v libx264 -preset veryslow -crf 30 -pix_fmt yuv420p -c:a aac -b:a 24k -shortest \
    -movflags +faststart -map_metadata -1 -fflags +bitexact -flags:v +bitexact -flags:a +bitexact "clip-$1.mp4"
}
make 1 "gradients=s=360x640:d=3:speed=0.08:nb_colors=4:seed=11"
make 2 "gradients=s=360x640:d=3:speed=0.12:nb_colors=6:seed=47:type=radial"
make 3 "gradients=s=360x640:d=3:speed=0.1:nb_colors=5:seed=83:type=spiral"
```
