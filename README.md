# unrpa-rs
This is a rust rewrite of [unrpa](https://github.com/Lattyware/unrpa). All credit goes to the original author & contributors.

There is no reason, why you should not just run the original. I just rewrote it because I dislike python and had nothing better to do.

It does not support `ZiX` archives and has no intention of ever doing so.
Displaying archive contents as a tree has also not been implemented yet, but may be in the future.
Otherwise it is able to correctly extract rpa archies.

## Usage

Please refer to the original unrpa documentation, or look at the output of `unrpa-rs --help`.

Noteably this rewrite is able to handle folders, not just rpa files as input.
That means you can just

```
unrpa-rs PATH_TO_RENPY_APP --mkdir -p APP_NAME
```

And in a few seconds you will have all rpa's within extracted into the `APP_NAME` folder in your current directory
