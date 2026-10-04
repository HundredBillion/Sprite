# Surface baseline

Base: `08f7c37`. Linux debug test harness; fixture setup outside allocation scopes. Counts include Rust allocation/reallocation requests on the calling thread, not whole-GPUI frame allocation. Initial render includes lazy process initialization.

```text
grid cell_bytes=32 create allocations=12061 bytes=397440
grid first render allocations=187 bytes=2205656
grid idle render allocations=122 bytes=579528
grid one-row update/render allocations=186 bytes=1157139
list parse/apply 100k allocations=400019 bytes=45471414
list 100k actual truncate_line calls=51
list replace with anchor 100k allocations=400021 bytes=47699660
list state 100k allocations=15 bytes=792
```

Commands:

```sh
TERM=xterm-ghostty cargo test --locked --offline -p sprite-app --lib surface_ -- --nocapture
TERM=xterm-ghostty /tmp/sprite-syscall-tools.B9rhrh/usr/bin/strace -f -e trace=write,writev,sendto,sendmsg -s 2048 -o /tmp/task8-wheel-before.trace target/debug/deps/sprite_app-b1d7a61a7846d134 surface_wheel_syscall_probe --nocapture --test-threads=1
```

Actual `TerminalView::report_grid_wheel`, established local Unix socket pair, empty receive buffer, 3 vertical + 2 horizontal cells, 399 total bytes: **10 sendto calls**, 5 JSON payload writes plus 5 newline writes between gesture markers. No authenticated endpoint or real credentials. This is a normal non-backpressured gesture, not a partial-write guarantee.
