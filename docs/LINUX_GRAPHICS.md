# Linux GUI graphics troubleshooting

## Intel Ivy Bridge on Hyprland / Wayland

Issue [#254](https://github.com/aula-id/koma/issues/254) reports intermittent
flicker or brief black frames on Intel HD Graphics with the i915 driver.
The report does not yet establish whether the problem affects only Koma or
the whole desktop; kernel and compositor logs are still needed to confirm
its cause.

Koma uses WebKitGTK for its Linux GUI. On Wayland sessions (`WAYLAND_DISPLAY`
is nonempty), Koma now selects WebKit's shared-memory buffer transport when
`/sys/class/drm/card*/device` identifies Intel Ivy Bridge (vendor `8086`,
devices `0152`, `0156`, `015a`, `0162`, `0166`, or `016a`). This avoids hardware
DMA-BUF import for those systems without changing Hyprland or i915 settings.
The fallback applies before GTK initializes and covers all Koma webviews.
It may increase CPU use / buffer copying. On multi-GPU systems, the presence
of an Ivy Bridge DRM card enables the fallback even if another GPU renders Koma.
Unreadable or missing sysfs entries leave the existing rendering behavior intact.

This is a compatibility mitigation, pending confirmation on the reported
hardware. WebKit's buffer transport is separate from its compositing policy;
Koma's existing compositing and View Transition safeguards still apply.
See the [WebKit transport selection implementation](https://github.com/WebKit/WebKit/blob/main/Source/WebKit/UIProcess/gtk/AcceleratedBackingStore.cpp).

To test the fallback manually, fully exit Koma's GUI and relaunch:

```sh
WEBKIT_DMABUF_RENDERER_FORCE_SHM=1 koma gui
```

To compare with the previous buffer selection:

```sh
WEBKIT_DMABUF_RENDERER_FORCE_SHM=0 koma gui
```

Explicit values of `WEBKIT_DMABUF_RENDERER_FORCE_SHM` or
`WEBKIT_DISABLE_DMABUF_RENDERER` are always preserved, including `0` and an
empty value. Remove those variables from the environment to restore automatic
selection. X11 sessions and other GPUs keep their existing defaults.

## If flicker continues

Compare scrolling, resizing, opening another Koma webview, and switching
workspaces with each command above. Record whether only Koma flickers or
whether the entire monitor briefly goes black. Include Koma and WebKitGTK
versions, GPU PCI IDs (`lspci -nnk`), and these diagnostics with the report:

```sh
hyprctl version
hyprctl monitors all
journalctl -b -k --no-pager | grep -Ei 'i915|drm|gpu|hang|reset|error'
```

Review logs for private information before sharing. If the whole display goes
black or i915 reports GPU resets, investigate the compositor / driver as well;
an application buffer fallback cannot establish or resolve every display-level
cause. Do not change kernel power-management or monitor settings without
checking those diagnostics first.
