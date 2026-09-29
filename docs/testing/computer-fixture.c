/* Disposable X11 fixture for the opt-in native adapter smoke test.
 * cc computer-fixture.c -lX11 -o computer-fixture
 * computer-fixture /tmp/fixture-state
 */
#include <X11/Xlib.h>
#include <X11/Xatom.h>
#include <X11/Xutil.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>
int main(int argc, char **argv) {
    if (argc != 2) return 2;
    Display *display = XOpenDisplay(NULL);
    if (!display) return 3;
    int screen = DefaultScreen(display);
    Window window = XCreateSimpleWindow(display, RootWindow(display, screen),
        100, 100, 420, 220, 0, 0, WhitePixel(display, screen));
    XStoreName(display, window, "Koma Native Fixture");
    unsigned long pid = (unsigned long)getpid();
    XChangeProperty(display, window, XInternAtom(display, "_NET_WM_PID", False),
        XA_CARDINAL, 32, PropModeReplace, (unsigned char *)&pid, 1);
    XSelectInput(display, window, ExposureMask | KeyPressMask | ButtonPressMask);
    GC gc = XCreateGC(display, window, 0, NULL);
    XMapWindow(display, window);
    XFlush(display);
    printf("%lu:%lu\n", window, pid);
    fflush(stdout);
    char text[256] = {0};
    unsigned int clicks = 0;
    for (;;) {
        XEvent event;
        XNextEvent(display, &event);
        if (event.type == KeyPress) {
            char key[16];
            int count = XLookupString(&event.xkey, key, sizeof(key), NULL, NULL);
            size_t length = strlen(text);
            if (count > 0 && length + (size_t)count < sizeof(text)) {
                memcpy(text + length, key, (size_t)count);
                text[length + (size_t)count] = 0;
            }
        }
        if (event.type == ButtonPress) clicks++;
        XSetForeground(display, gc, WhitePixel(display, screen));
        XFillRectangle(display, window, gc, 0, 0, 420, 220);
        XSetForeground(display, gc, BlackPixel(display, screen));
        XDrawString(display, window, gc, 20, 35, "Koma Native Fixture", 19);
        XDrawString(display, window, gc, 20, 70, text, (int)strlen(text));
        XFillRectangle(display, window, gc, 20, 100, 10 + (unsigned int)strlen(text) * 10, 20);
        XFlush(display);
        FILE *state = fopen(argv[1], "w");
        if (state) { fprintf(state, "%u\n%s", clicks, text); fclose(state); }
    }
}
