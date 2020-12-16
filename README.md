
#### What and why?

The crate contains a silly tool `sb`, which pivots to a new rootfs and runs a
process.

This is just an experiment that exercises some of the namespacing and mount APIs
though an unsafe wrappers provided by the [libc](https://crates.io/crates/libc)
crate.

#### Build & run

To build and run:

```
$ cargo build
$ target/debug/sb <rootfs> <process>
```

Start a bash process from the Core 20 base snap:

```
$ target/debug/sb /snap/core20/current /bin/bash
```
