// macOS GUI-only SDK bridge. Captures serve model observations or ephemeral live
// preview frames. Only model observations run enrichment. Desktop strings are data.
#import <AppKit/AppKit.h>
#import <ApplicationServices/ApplicationServices.h>
#import <ImageIO/ImageIO.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>
#import <Vision/Vision.h>
#include "capture_limits.h"
#include "input_idle.h"
#include <libproc.h>
#include <sys/sysctl.h>
#include <algorithm>
#include <atomic>
#include <chrono>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <stdexcept>
#include <thread>
#include <unistd.h>
#include <vector>

// One dotted decimal component. Stops at the first non-digit.
static bool parse_os_component(const char *&p, unsigned &out) {
    if (*p < '0' || *p > '9')
        return false;
    unsigned value = 0;
    while (*p >= '0' && *p <= '9') {
        value = value * 10u + static_cast<unsigned>(*p - '0');
        ++p;
    }
    out = value;
    return true;
}

// Clang lowers `@available` to ___isPlatformVersionAtLeast. That helper lives
// in libclang_rt.osx.a, which rustc does not link for an object built by
// cc-rs, so `cargo build --release` fails with an undefined symbol. Weak so a
// toolchain copy wins when one is present. Platform 1 is macOS.
//
// Do not call NSProcessInfo here. isOperatingSystemAtLeastVersion: is itself
// implemented with @available, which calls this function and overflows the
// stack the first time computer use checks macOS 14.
extern "C" __attribute__((weak)) int32_t __isPlatformVersionAtLeast(uint32_t platform, uint32_t major,
                                                                    uint32_t minor, uint32_t subminor) {
    if (platform != 1)
        return 0;
    char buf[64];
    std::memset(buf, 0, sizeof(buf));
    size_t len = sizeof(buf);
    if (sysctlbyname("kern.osproductversion", buf, &len, nullptr, 0) != 0)
        return 0;
    buf[sizeof(buf) - 1] = '\0';
    const char *p = buf;
    unsigned have_maj = 0, have_min = 0, have_sub = 0;
    if (!parse_os_component(p, have_maj))
        return 0;
    if (*p == '.') {
        ++p;
        if (!parse_os_component(p, have_min))
            return 0;
    }
    if (*p == '.') {
        ++p;
        if (!parse_os_component(p, have_sub))
            return 0;
    }
    if (have_maj != major)
        return have_maj > major ? 1 : 0;
    if (have_min != minor)
        return have_min > minor ? 1 : 0;
    return have_sub >= subminor ? 1 : 0;
}

static std::atomic<bool> cancelled(false);
static NSDictionary *target;
static std::vector<CGKeyCode> held;
static CGEventFlags flags = 0;
static bool inputStarted = false;
static void postInputEvent(CGEventRef event) {
    inputStarted = true;
    CGEventPost(kCGHIDEventTap, event);
}
static void require(bool condition, const char *message) {
    if (!condition)
        throw std::runtime_error(message);
}
static void check() { require(!cancelled.load(), "Computer operation cancelled"); }
static NSDictionary *rect(CGRect r) {
    return @{
        @"x" : @(r.origin.x),
        @"y" : @(r.origin.y),
        @"width" : @(r.size.width),
        @"height" : @(r.size.height)
    };
}
static CGRect bounds(NSDictionary *r) {
    return CGRectMake([r[@"x"] doubleValue], [r[@"y"] doubleValue], [r[@"width"] doubleValue],
                      [r[@"height"] doubleValue]);
}
static NSString *limited(NSString *s) {
    if (![s isKindOfClass:NSString.class])
        return @"";
    if (s.length <= 256)
        return s;
    NSUInteger end = 256;
    if (CFStringIsSurrogateHighCharacter([s characterAtIndex:end - 1]))
        --end;
    return [s substringToIndex:end];
}
static id attr(AXUIElementRef e, CFStringRef key) {
    CFTypeRef value = nullptr;
    if (AXUIElementCopyAttributeValue(e, key, &value) != kAXErrorSuccess)
        return nil;
    return CFBridgingRelease(value);
}
static CGRect axBounds(AXUIElementRef e) {
    id p = attr(e, kAXPositionAttribute), s = attr(e, kAXSizeAttribute);
    CGPoint point = {};
    CGSize size = {};
    if (p && s && CFGetTypeID((__bridge CFTypeRef)p) == AXValueGetTypeID() &&
        CFGetTypeID((__bridge CFTypeRef)s) == AXValueGetTypeID()) {
        AXValueGetValue((__bridge AXValueRef)p, kAXValueTypeCGPoint, &point);
        AXValueGetValue((__bridge AXValueRef)s, kAXValueTypeCGSize, &size);
    }
    return CGRectMake(point.x, point.y, size.width, size.height);
}
static bool sameRect(CGRect a, CGRect b) {
    return fabs(a.origin.x - b.origin.x) < 1 && fabs(a.origin.y - b.origin.y) < 1 &&
           fabs(a.size.width - b.size.width) < 1 && fabs(a.size.height - b.size.height) < 1;
}
static NSArray *windowInfo() {
    return CFBridgingRelease(CGWindowListCopyWindowInfo(
        kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements, kCGNullWindowID));
}
// Discovery and capture must use the same set of shareable windows. Quartz
// window titles are optional; a missing title must not hide an application.
static SCShareableContent *shareableContent() API_AVAILABLE(macos(14.0)) {
    check();
    require(CGPreflightScreenCaptureAccess(),
            "Grant Screen Recording access and reactivate control");
    __block SCShareableContent *content = nil;
    __block NSError *error = nil;
    dispatch_semaphore_t ready = dispatch_semaphore_create(0);
    [SCShareableContent
        getShareableContentExcludingDesktopWindows:YES
                               onScreenWindowsOnly:YES
                                 completionHandler:^(SCShareableContent *c, NSError *e) {
                                   content = c;
                                   error = e;
                                   dispatch_semaphore_signal(ready);
                                 }];
    auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds(5);
    while (dispatch_semaphore_wait(ready, dispatch_time(DISPATCH_TIME_NOW, 20 * NSEC_PER_MSEC)) != 0) {
        check();
        require(std::chrono::steady_clock::now() < deadline, "Window discovery timed out");
    }
    check();
    require(!error && content, "ScreenCaptureKit window discovery failed");
    return content;
}
static NSDictionary *lookup(NSString *identity) {
    NSArray *parts = [identity componentsSeparatedByString:@":"];
    require(parts.count == 2, "Invalid macOS window identity");
    uint32_t wid = [parts[0] intValue];
    pid_t pid = [parts[1] intValue];
    require(pid > 0 && pid != getpid(), "Cannot target Koma");
    for (NSDictionary *w in windowInfo()) {
        if ([w[(id)kCGWindowNumber] unsignedIntValue] == wid &&
            [w[(id)kCGWindowOwnerPID] intValue] == pid)
            return w;
    }
    throw std::runtime_error("Selected window closed, hidden, or changed identity");
}
static CGRect windowBounds(NSDictionary *w) {
    CGRect r = {};
    CGRectMakeWithDictionaryRepresentation((__bridge CFDictionaryRef)w[(id)kCGWindowBounds], &r);
    require(r.size.width > 0 && r.size.height > 0, "Invalid window geometry");
    return r;
}
// Display sources use global Quartz coordinates (including negative origins).
static bool isDisplay(NSString *identity) { return [identity hasPrefix:@"display:"]; }
static CGDirectDisplayID displayID(NSString *identity) {
    require(isDisplay(identity), "Select a display before desktop input");
    unsigned long long value = [[identity substringFromIndex:8] longLongValue];
    require(value > 0 && value <= UINT32_MAX, "Invalid display identity");
    CGDirectDisplayID display = (CGDirectDisplayID)value;
    require(CGDisplayIsActive(display), "Shared display disconnected; select a display again");
    return display;
}
static NSString *desktopFocus() {
    NSRunningApplication *app = NSWorkspace.sharedWorkspace.frontmostApplication;
    if (!app) return @"none";
    AXUIElementRef ax = AXUIElementCreateApplication(app.processIdentifier);
    AXUIElementSetMessagingTimeout(ax, 0.1f);
    id focusedWindow = attr(ax, kAXFocusedWindowAttribute);
    CFRelease(ax);
    CGRect r = focusedWindow ? axBounds((__bridge AXUIElementRef)focusedWindow) : CGRectZero;
    return [NSString stringWithFormat:@"%d:%.0f:%.0f:%.0f:%.0f:%@", app.processIdentifier,
        r.origin.x, r.origin.y, r.size.width, r.size.height,
        focusedWindow ? limited(attr((__bridge AXUIElementRef)focusedWindow, kAXTitleAttribute)) : @""];
}
static NSDictionary *describeDisplay(CGDirectDisplayID display) {
    return @{
        @"id": [NSString stringWithFormat:@"display:%u", display],
        @"application": @"Desktop",
        @"title": [NSString stringWithFormat:@"Display %u%@", display, display == CGMainDisplayID() ? @" · Main" : @""],
        @"geometry": rect(CGDisplayBounds(display)), @"focused": @YES,
        @"focus": desktopFocus()
    };
}
static void guardKeyboard() {
    if (!isDisplay(target[@"id"])) return;
    // Title, bounds, and window identity are not a batch gate. A predicted
    // chord or type keeps going when focus moves to another window on this
    // display. A missing foreground window is not a failure: X11 falls back
    // to the root, and Windows ignores a null foreground the same way.
    NSRunningApplication *app = NSWorkspace.sharedWorkspace.frontmostApplication;
    if (!app) return;
    AXUIElementRef ax = AXUIElementCreateApplication(app.processIdentifier);
    AXUIElementSetMessagingTimeout(ax, 0.1f);
    id focusedWindow = attr(ax, kAXFocusedWindowAttribute);
    CFRelease(ax);
    if (focusedWindow) {
        CGRect r = axBounds((__bridge AXUIElementRef)focusedWindow);
        require(CGRectContainsPoint(bounds(target[@"geometry"]), CGPointMake(CGRectGetMidX(r), CGRectGetMidY(r))),
                "Keyboard focus is outside the shared display; click a visible window and observe again");
    }
}
// Retained selected-window AX reference. Match only the selected process and
// exact window geometry/title, and reject ambiguous application windows.
static AXUIElementRef axWindow(NSDictionary *w) {
    require(AXIsProcessTrusted(), "Grant Accessibility access to Koma and reactivate control");
    AXUIElementRef app = AXUIElementCreateApplication([w[(id)kCGWindowOwnerPID] intValue]);
    AXUIElementSetMessagingTimeout(app, 0.1f);
    CFIndex count = 0;
    AXUIElementGetAttributeValueCount(app, kAXWindowsAttribute, &count);
    CFArrayRef raw = nullptr;
    AXUIElementCopyAttributeValues(app, kAXWindowsAttribute, 0, std::min<CFIndex>(count, 64), &raw);
    CFRelease(app);
    require(raw, "Application does not expose accessibility windows");
    NSArray *windows = CFBridgingRelease(raw);
    AXUIElementRef found = nullptr;
    auto deadline = std::chrono::steady_clock::now() + std::chrono::milliseconds(750);
    for (id object in windows) {
        check();
        require(std::chrono::steady_clock::now() < deadline,
                "Accessibility window matching time limit reached");
        AXUIElementRef e = (__bridge AXUIElementRef)object;
        AXUIElementSetMessagingTimeout(e, 0.1f);
        if (sameRect(axBounds(e), windowBounds(w)) &&
            [attr(e, kAXTitleAttribute) isEqual:w[(id)kCGWindowName] ?: @""]) {
            require(found == nullptr, "Ambiguous accessibility window identity");
            found = e;
        }
    }
    require(found, "Cannot uniquely match the selected accessibility window");
    CFRetain(found);
    return found;
}
static bool focused(NSDictionary *w) {
    if (!AXIsProcessTrusted())
        return false;
    AXUIElementRef app = AXUIElementCreateApplication([w[(id)kCGWindowOwnerPID] intValue]);
    AXUIElementSetMessagingTimeout(app, 0.1f);
    id front = attr(app, kAXFocusedWindowAttribute);
    CFRelease(app);
    if (NSWorkspace.sharedWorkspace.frontmostApplication.processIdentifier !=
        [w[(id)kCGWindowOwnerPID] intValue])
        return false;
    if (!front)
        return false;
    // Accessibility bounds and the window-list bounds often disagree by the
    // title bar. A one-pixel match rejected Chrome even when it was in front.
    CGRect ax = axBounds((__bridge AXUIElementRef)front);
    CGRect cg = windowBounds(w);
    const CGFloat slop = 28;
    bool near = fabs(ax.origin.x - cg.origin.x) < slop && fabs(ax.origin.y - cg.origin.y) < slop &&
                fabs(ax.size.width - cg.size.width) < slop && fabs(ax.size.height - cg.size.height) < slop;
    CGPoint center = CGPointMake(CGRectGetMidX(ax), CGRectGetMidY(ax));
    return near || CGRectContainsPoint(CGRectInset(cg, -slop, -slop), center);
}
static NSDictionary *describe(NSDictionary *w) {
    NSString *application = limited(w[(id)kCGWindowOwnerName]);
    if (!application.length) {
        pid_t pid = [w[(id)kCGWindowOwnerPID] intValue];
        application = limited([NSRunningApplication runningApplicationWithProcessIdentifier:pid].localizedName);
        if (!application.length) application = [NSString stringWithFormat:@"Application %d", pid];
    }
    NSString *title = limited(w[(id)kCGWindowName]);
    return @{
        @"id" :
            [NSString stringWithFormat:@"%@:%@", w[(id)kCGWindowNumber], w[(id)kCGWindowOwnerPID]],
        @"application" : application,
        @"title" : title.length ? title : application,
        @"geometry" : rect(windowBounds(w)),
        @"focused" : @(focused(w))
    };
}
static void unobstructed(NSDictionary *w) {
    CGRect r = windowBounds(w);
    bool found = false;
    for (NSDictionary *above in windowInfo()) {
        if ([above[(id)kCGWindowNumber] isEqual:w[(id)kCGWindowNumber]]) {
            found = true;
            break;
        }
        // The enlarged arrow is our own window. It overlaps the target on
        // purpose and must not count as something covering it.
        if ([above[(id)kCGWindowOwnerPID] intValue] == NSProcessInfo.processInfo.processIdentifier)
            continue;
        if ([above[(id)kCGWindowAlpha] doubleValue] <= 0)
            continue;
        CGRect overlap = CGRectIntersection(r, windowBounds(above));
        require(CGRectIsNull(overlap) || CGRectIsEmpty(overlap),
                "The application window is still covered; observe again");
    }
    require(found, "Target no longer visible");
}
static void raiseApplication(NSDictionary *w) {
    pid_t pid = [w[(id)kCGWindowOwnerPID] intValue];
    NSRunningApplication *app = [NSRunningApplication runningApplicationWithProcessIdentifier:pid];
    if (app)
        [app activateWithOptions:NSApplicationActivateIgnoringOtherApps];
    if (!AXIsProcessTrusted())
        return;
    try {
        AXUIElementRef ax = axWindow(w);
        AXUIElementPerformAction(ax, kAXRaiseAction);
        CFRelease(ax);
    } catch (const std::exception &) {
    }
}
static void guardInput() {
    check();
    waitForInputIdle(check, [] {
        for (CGKeyCode code = 0; code < 128; ++code)
            if (CGEventSourceKeyState(kCGEventSourceStateCombinedSessionState, code) &&
                std::find(held.begin(), held.end(), code) == held.end()) return true;
        for (uint32_t button = 0; button < 5; ++button)
            if (CGEventSourceButtonState(kCGEventSourceStateCombinedSessionState,
                                        static_cast<CGMouseButton>(button))) return true;
        return false;
    });
    require(target != nil, "No input target");
    require(CGPreflightScreenCaptureAccess() && AXIsProcessTrusted(), "Desktop permissions revoked; reactivate control");
    if (isDisplay(target[@"id"])) {
        // Still connected. Screenshot points are mapped onto the current
        // bounds, so a move or resize does not cancel the batch.
        (void)displayID(target[@"id"]);
    } else {
        NSDictionary *w = lookup(target[@"id"]);
        raiseApplication(w);
        auto deadline = std::chrono::steady_clock::now() + std::chrono::milliseconds(300);
        while (!focused(w) && std::chrono::steady_clock::now() < deadline) {
            check();
            std::this_thread::sleep_for(std::chrono::milliseconds(20));
        }
        require(focused(w), "Could not bring the application window forward; observe again");
        unobstructed(w);
    }
}
static const CGFloat kArrow = 36;
static const CGFloat kHot = 2;
static NSPanel *cursorPanel = nil;
static bool arrowShown = false;
static void hideArrowMain() { [cursorPanel orderOut:nil]; }
static void onMain(void (^block)(void)) {
    if ([NSThread isMainThread])
        block();
    else
        dispatch_sync(dispatch_get_main_queue(), block);
}
static void hideArrow() {
    if (!arrowShown)
        return;
    onMain(^{ hideArrowMain(); });
    arrowShown = false;
}
static void ensureArrowMain() {
    if (cursorPanel)
        return;
    cursorPanel = [[NSPanel alloc] initWithContentRect:NSMakeRect(0, 0, kArrow, kArrow)
                                              styleMask:NSWindowStyleMaskBorderless
                                                backing:NSBackingStoreBuffered
                                                  defer:NO];
    cursorPanel.opaque = NO;
    cursorPanel.backgroundColor = NSColor.clearColor;
    cursorPanel.ignoresMouseEvents = YES;
    cursorPanel.level = NSStatusWindowLevel;
    cursorPanel.hidesOnDeactivate = NO;
    cursorPanel.collectionBehavior = NSWindowCollectionBehaviorCanJoinAllSpaces |
                                     NSWindowCollectionBehaviorStationary |
                                     NSWindowCollectionBehaviorFullScreenAuxiliary |
                                     NSWindowCollectionBehaviorIgnoresCycle;
    [cursorPanel setSharingType:NSWindowSharingNone];
    NSImage *image = [[NSImage alloc] initWithSize:NSMakeSize(kArrow, kArrow)];
    [image lockFocus];
    NSAffineTransform *flip = [NSAffineTransform transform];
    [flip translateXBy:0 yBy:kArrow];
    [flip scaleXBy:1 yBy:-1];
    [flip concat];
    NSBezierPath *path = [NSBezierPath bezierPath];
    CGFloat pts[][2] = {{2, 2}, {2, 28}, {9, 21}, {14, 32}, {19, 29}, {13, 19}, {22, 19}};
    [path moveToPoint:NSMakePoint(pts[0][0], pts[0][1])];
    for (int i = 1; i < 7; ++i)
        [path lineToPoint:NSMakePoint(pts[i][0], pts[i][1])];
    [path closePath];
    [NSColor.whiteColor setFill];
    [path fill];
    [NSColor.blackColor setStroke];
    path.lineWidth = 1.5;
    [path stroke];
    [image unlockFocus];
    NSImageView *view = [[NSImageView alloc] initWithFrame:NSMakeRect(0, 0, kArrow, kArrow)];
    view.image = image;
    cursorPanel.contentView = view;
}
static void placeArrow(CGPoint point) {
    onMain(^{
        ensureArrowMain();
        CGFloat height = NSScreen.screens.firstObject.frame.size.height;
        [cursorPanel setFrameOrigin:NSMakePoint(point.x - kHot, height - (point.y - kHot) - kArrow)];
        [cursorPanel orderFrontRegardless];
    });
    arrowShown = true;
}
static CGPoint currentPointer() {
    CGEventRef event = CGEventCreate(nullptr);
    CGPoint point = event ? CGEventGetLocation(event) : CGPointZero;
    if (event)
        CFRelease(event);
    return point;
}
static void movePointer(CGPoint point) {
    CGEventRef move = CGEventCreateMouseEvent(nullptr, kCGEventMouseMoved, point, kCGMouseButtonLeft);
    require(move, "Unable to move pointer");
    postInputEvent(move);
    CFRelease(move);
}
static void playGlide(NSArray *steps, CGPoint target) {
    if (![steps isKindOfClass:NSArray.class] || steps.count == 0) {
        movePointer(target);
        return;
    }
    CGPoint previous = currentPointer();
    for (NSUInteger i = 0; i < steps.count; ++i) {
        check();
        NSArray *pair = steps[i];
        require([pair isKindOfClass:NSArray.class] && pair.count == 2, "Invalid glide step");
        CGPoint point = CGPointMake([pair[0] doubleValue], [pair[1] doubleValue]);
        movePointer(point);
        placeArrow(point);
        CGPoint now = currentPointer();
        double dx = now.x - point.x, dy = now.y - point.y;
        double lx = now.x - previous.x, ly = now.y - previous.y;
        // A sample that has not been applied yet still sits on the previous
        // point. Only a position away from both is the hand taking the mouse.
        if (dx * dx + dy * dy > 144.0 && lx * lx + ly * ly > 144.0) {
            movePointer(target);
            placeArrow(target);
            return;
        }
        previous = point;
        if (i + 1 < steps.count) {
            for (int s = 0; s < 2; ++s) {
                check();
                std::this_thread::sleep_for(std::chrono::milliseconds(8));
            }
        }
    }
}
static void release() {
    hideArrow();
    for (auto it = held.rbegin(); it != held.rend(); ++it) {
        CGEventRef e = CGEventCreateKeyboardEvent(nullptr, *it, false);
        if (e) {
            CGEventSetFlags(e, 0);
            CGEventPost(kCGHIDEventTap, e);
            CFRelease(e);
        }
    }
    held.clear();
    flags = 0;
}
static CGKeyCode keyCode(NSString *s) {
    NSDictionary *keys = @{
        @"Return" : @36,
        @"Enter" : @36,
        @"Tab" : @48,
        @"Space" : @49,
        @"space" : @49,
        @"Escape" : @53,
        @"Esc" : @53,
        @"Backspace" : @51,
        @"BackSpace" : @51,
        @"Delete" : @117,
        @"Home" : @115,
        @"End" : @119,
        @"PageUp" : @116,
        @"PageDown" : @121,
        @"Left" : @123,
        @"ArrowLeft" : @123,
        @"Right" : @124,
        @"ArrowRight" : @124,
        @"Down" : @125,
        @"ArrowDown" : @125,
        @"Up" : @126,
        @"ArrowUp" : @126,
        @"Shift" : @56,
        @"Shift_L" : @56,
        @"Shift_R" : @60,
        @"Control" : @59,
        @"Ctrl" : @59,
        @"Control_L" : @59,
        @"Control_R" : @62,
        @"Alt" : @58,
        @"Alt_L" : @58,
        @"Alt_R" : @61,
        @"Super" : @55,
        @"Command" : @55,
        @"Super_L" : @55,
        @"Super_R" : @54,
        @"Meta" : @55,
        @"Meta_L" : @55,
        @"Meta_R" : @54,
        @"a" : @0,
        @"s" : @1,
        @"d" : @2,
        @"f" : @3,
        @"h" : @4,
        @"g" : @5,
        @"z" : @6,
        @"x" : @7,
        @"c" : @8,
        @"v" : @9,
        @"b" : @11,
        @"q" : @12,
        @"w" : @13,
        @"e" : @14,
        @"r" : @15,
        @"y" : @16,
        @"t" : @17,
        @"1" : @18,
        @"2" : @19,
        @"3" : @20,
        @"4" : @21,
        @"6" : @22,
        @"5" : @23,
        @"9" : @25,
        @"7" : @26,
        @"8" : @28,
        @"0" : @29,
        @"o" : @31,
        @"u" : @32,
        @"i" : @34,
        @"p" : @35,
        @"l" : @37,
        @"j" : @38,
        @"k" : @40,
        @"n" : @45,
        @"m" : @46,
        @"." : @47, @"," : @43, @"/" : @44, @";" : @41, @"'" : @39,
        @"[" : @33, @"]" : @30, @"\\" : @42, @"-" : @27, @"=" : @24, @"`" : @50,
        @">" : @47, @"<" : @43, @"?" : @44, @":" : @41, @"\"" : @39,
        @"{" : @33, @"}" : @30, @"|" : @42, @"_" : @27, @"+" : @24, @"~" : @50,
        @"!" : @18, @"@" : @19, @"#" : @20, @"$" : @21, @"%" : @23, @"^" : @22,
        @"&" : @26, @"*" : @28, @"(" : @25, @")" : @29,
        @"F1" : @122,
        @"F2" : @120,
        @"F3" : @99,
        @"F4" : @118,
        @"F5" : @96,
        @"F6" : @97,
        @"F7" : @98,
        @"F8" : @100,
        @"F9" : @101,
        @"F10" : @109,
        @"F11" : @103,
        @"F12" : @111
    };
    NSNumber *code = keys[s] ?: keys[s.lowercaseString];
    require(code != nil, "key is not available: unsupported macOS key name");
    return (CGKeyCode)code.unsignedShortValue;
}
static CGEventFlags modifier(CGKeyCode key) {
    if (key == 56 || key == 60)
        return kCGEventFlagMaskShift;
    if (key == 59 || key == 62)
        return kCGEventFlagMaskControl;
    if (key == 58 || key == 61)
        return kCGEventFlagMaskAlternate;
    if (key == 54 || key == 55)
        return kCGEventFlagMaskCommand;
    return 0;
}
static void key(CGKeyCode code, bool down) {
    if (down) {
        guardInput();
        held.push_back(code);
        flags |= modifier(code);
    } else
        flags &= ~modifier(code);
    CGEventRef e = CGEventCreateKeyboardEvent(nullptr, code, down);
    require(e, "Unable to create keyboard event");
    CGEventSetFlags(e, flags);
    postInputEvent(e);
    CFRelease(e);
}
static CGPoint mapPoint(NSDictionary *a, NSDictionary *t) {
    CGRect r = bounds(t[@"desktop"]);
    double x = [a[@"x"] doubleValue], y = [a[@"y"] doubleValue];
    double width = [t[@"width"] doubleValue], height = [t[@"height"] doubleValue];
    require(x >= 0 && y >= 0 && x < width && y < height, "Invalid screenshot coordinate");
    CGPoint point = CGPointMake(r.origin.x + x * r.size.width / width,
                                r.origin.y + y * r.size.height / height);
    require(CGRectContainsPoint(bounds(target[@"geometry"]), point), "Input outside shared source");
    return point;
}
static void input(NSDictionary *a, NSDictionary *t, NSArray *glide) {
    guardInput();
    NSString *kind = a[@"kind"];
    if ([kind isEqual:@"type"]) {
        guardKeyboard();
        NSString *text = a[@"text"];
        for (NSUInteger i = 0; i < text.length;) {
            guardInput();
            guardKeyboard();
            NSRange range = [text rangeOfComposedCharacterSequenceAtIndex:i];
            // CGEvent carries UTF-16 directly; no layout mutation or clipboard replacement.
            NSString *chunk = [text substringWithRange:range];
            require(chunk.length <= 64, "Text grapheme exceeds native event limit");
            if ([chunk isEqual:@"\n"] || [chunk isEqual:@"\r"] || [chunk isEqual:@"\t"]) {
                key([chunk isEqual:@"\t"] ? 48 : 36, true);
                release();
                i = NSMaxRange(range);
                continue;
            }
            UniChar chars[64];
            [chunk getCharacters:chars range:NSMakeRange(0, chunk.length)];
            CGEventRef down = CGEventCreateKeyboardEvent(nullptr, 0, true);
            CGEventRef up = CGEventCreateKeyboardEvent(nullptr, 0, false);
            require(down && up, "Unable to create Unicode input");
            CGEventKeyboardSetUnicodeString(down, chunk.length, chars);
            CGEventKeyboardSetUnicodeString(up, chunk.length, chars);
            CGEventSetFlags(down, 0);
            CGEventSetFlags(up, 0);
            // Unicode events use virtual key 0. Track our carrier just like a
            // chord key so the next grapheme cannot mistake its queued down
            // event for a physical key press. Cleanup also covers cancellation.
            if (std::find(held.begin(), held.end(), 0) == held.end()) held.push_back(0);
            postInputEvent(down);
            postInputEvent(up);
            CFRelease(down);
            CFRelease(up);
            i = NSMaxRange(range);
        }
        release();
    } else if ([kind isEqual:@"key"]) {
        NSArray *keyNames = a[@"keys"];
        // A lone punctuation mark is a character, same as type. A chord such
        // as Control+. still uses the keycode below.
        if (keyNames.count == 1) {
            NSString *only = keyNames.firstObject;
            if (only.length == 1) {
                unichar c = [only characterAtIndex:0];
                if (c >= 33 && c < 127 && ![[NSCharacterSet alphanumericCharacterSet] characterIsMember:c]) {
                    guardKeyboard();
                    UniChar chars[1] = {c};
                    CGEventRef down = CGEventCreateKeyboardEvent(nullptr, 0, true);
                    CGEventRef up = CGEventCreateKeyboardEvent(nullptr, 0, false);
                    require(down && up, "Unable to create Unicode input");
                    CGEventKeyboardSetUnicodeString(down, 1, chars);
                    CGEventKeyboardSetUnicodeString(up, 1, chars);
                    CGEventSetFlags(down, 0);
                    CGEventSetFlags(up, 0);
                    postInputEvent(down);
                    postInputEvent(up);
                    CFRelease(down);
                    CFRelease(up);
                    release();
                    return;
                }
            }
        }
        guardKeyboard();
        std::vector<CGKeyCode> codes;
        for (NSString *s in keyNames)
            codes.push_back(keyCode(s));
        for (auto code : codes)
            key(code, true);
        release();
    } else {
        CGPoint p = mapPoint(a, t);
        struct ArrowGuard {
            ~ArrowGuard() { hideArrow(); }
        } guard;
        (void)guard;
        playGlide(glide, p);
        if ([kind isEqual:@"click"]) {
            bool right = [a[@"button"] isEqual:@"right"];
            int count = [a[@"button"] isEqual:@"double"] ? 2 : 1;
            for (int i = 1; i <= count; ++i) {
                guardInput();
                CGMouseButton b = right ? kCGMouseButtonRight : kCGMouseButtonLeft;
                CGEventRef down = CGEventCreateMouseEvent(
                    nullptr, right ? kCGEventRightMouseDown : kCGEventLeftMouseDown, p, b);
                CGEventRef up = CGEventCreateMouseEvent(
                    nullptr, right ? kCGEventRightMouseUp : kCGEventLeftMouseUp, p, b);
                require(down && up, "Unable to click");
                CGEventSetIntegerValueField(down, kCGMouseEventClickState, i);
                CGEventSetIntegerValueField(up, kCGMouseEventClickState, i);
                postInputEvent(down);
                postInputEvent(up);
                CFRelease(down);
                CFRelease(up);
            }
        } else if ([kind isEqual:@"scroll"]) {
            guardInput();
            CGEventRef e = CGEventCreateScrollWheelEvent(nullptr, kCGScrollEventUnitLine, 1,
                                                         -[a[@"delta"] intValue]);
            require(e, "Unable to scroll");
            postInputEvent(e);
            CFRelease(e);
        } else
            require([kind isEqual:@"move"], "Unsupported input action");
    }
}
static void accessibility(NSDictionary *w, CGRect desktop, size_t width, size_t height,
                          NSMutableArray *elements) {
    AXUIElementRef selected = axWindow(w);
    id root = CFBridgingRelease(selected);
    NSMutableArray *queue = [NSMutableArray arrayWithObject:@[ root, @0 ]];
    auto deadline = std::chrono::steady_clock::now() + std::chrono::milliseconds(750);
    unsigned visited = 0;
    while (queue.count && visited++ < 256) {
        check();
        require(std::chrono::steady_clock::now() < deadline,
                "Accessibility extraction time limit reached");
        NSArray *item = queue[0];
        [queue removeObjectAtIndex:0];
        AXUIElementRef e = (__bridge AXUIElementRef)item[0];
        int depth = [item[1] intValue];
        AXUIElementSetMessagingTimeout(e, 0.1f);
        NSString *role = attr(e, kAXRoleAttribute), *subrole = attr(e, kAXSubroleAttribute);
        if ([subrole isEqual:@"AXSecureTextField"] || [attr(e, CFSTR("AXHidden")) boolValue])
            continue;
        CGRect r = axBounds(e);
        CGRect local = CGRectMake((r.origin.x - desktop.origin.x) * width / desktop.size.width,
                                  (r.origin.y - desktop.origin.y) * height / desktop.size.height,
                                  r.size.width * width / desktop.size.width,
                                  r.size.height * height / desktop.size.height);
        if (r.size.width > 0 && r.size.height > 0 &&
            CGRectContainsRect(CGRectMake(0, 0, width, height), local)) {
            NSString *label = attr(e, kAXTitleAttribute) ?: attr(e, kAXDescriptionAttribute) ?: @"";
            NSString *normalized =
                [role isEqual:@"AXTextField"]
                    ? @"entry"
                    : ([role isEqual:@"AXTextArea"] ? @"editable text" : limited(role));
            [elements addObject:@{
                @"id" : @"",
                @"source" : @"accessibility",
                @"label" : limited(label),
                @"role" : normalized,
                @"bounds" : rect(local),
                @"enabled" : @([attr(e, kAXEnabledAttribute) boolValue]),
                @"selected" : @([attr(e, kAXSelectedAttribute) boolValue]),
                @"focused" : @([attr(e, kAXFocusedAttribute) boolValue]),
                @"confidence" : NSNull.null
            }];
        }
        if (depth >= 12)
            continue;
        CFIndex count = 0;
        AXUIElementGetAttributeValueCount(e, kAXChildrenAttribute, &count);
        CFArrayRef children = nullptr;
        CFIndex limit = std::min<CFIndex>(count, 256 - visited - (unsigned)queue.count);
        if (limit > 0 && AXUIElementCopyAttributeValues(e, kAXChildrenAttribute, 0, limit,
                                                        &children) == kAXErrorSuccess) {
            for (id child in CFBridgingRelease(children))
                [queue addObject:@[ child, @(depth + 1) ]];
        }
    }
}
static NSDictionary *capture(NSString *identity, bool enrich, NSDictionary *region) API_AVAILABLE(macos(14.0)) {
    check();
    bool displaySource = isDisplay(identity);
    CGDirectDisplayID display = displaySource ? displayID(identity) : kCGNullDirectDisplay;
    NSDictionary *w = displaySource ? nil : lookup(identity);
    CGRect desktop = displaySource ? CGDisplayBounds(display) : windowBounds(w);
    SCShareableContent *content = shareableContent();
    SCContentFilter *filter = nil;
    if (displaySource) {
        SCDisplay *selected = nil;
        for (SCDisplay *candidate in content.displays)
            if (candidate.displayID == display) selected = candidate;
        require(selected, "Selected display no longer shareable");
        filter = [[SCContentFilter alloc] initWithDisplay:selected excludingWindows:@[]];
    } else {
        SCWindow *selected = nil;
        for (SCWindow *candidate in content.windows)
            if (candidate.windowID == [w[(id)kCGWindowNumber] unsignedIntValue]) selected = candidate;
        require(selected, "Selected window no longer shareable");
        filter = [[SCContentFilter alloc] initWithDesktopIndependentWindow:selected];
    }
    SCStreamConfiguration *configuration = [SCStreamConfiguration new];
    CGRect sourceDesktop = desktop;
    if (region) {
        require(displaySource && [region isKindOfClass:NSDictionary.class], "Region capture requires a display");
        CGRect selectedRegion = bounds(region);
        require(selectedRegion.size.width > 0 && selectedRegion.size.height > 0 && CGRectContainsRect(desktop, selectedRegion), "Region outside shared display");
        configuration.sourceRect = CGRectMake(selectedRegion.origin.x - desktop.origin.x,
            selectedRegion.origin.y - desktop.origin.y, selectedRegion.size.width, selectedRegion.size.height);
        sourceDesktop = selectedRegion;
    }
    auto size = captureSize((size_t)llround(sourceDesktop.size.width * filter.pointPixelScale),
                            (size_t)llround(sourceDesktop.size.height * filter.pointPixelScale), !enrich);
    configuration.width = size.first;
    configuration.height = size.second;
    configuration.scalesToFit = YES;
    require(configuration.width > 0 && configuration.height > 0 &&
                configuration.width * configuration.height <= 32000000,
            "Capture dimensions exceed limit");
    configuration.showsCursor = !enrich;
    configuration.ignoreShadowsSingleWindow = YES;
    configuration.shouldBeOpaque = YES;
    __block id capturedImage = nil;
    __block NSError *error = nil;
    dispatch_semaphore_t ready = dispatch_semaphore_create(0);
    [SCScreenshotManager captureImageWithFilter:filter
                                  configuration:configuration
                              completionHandler:^(CGImageRef img, NSError *e) {
                                if (img)
                                    capturedImage = CFBridgingRelease(CGImageRetain(img));
                                error = e;
                                dispatch_semaphore_signal(ready);
                              }];
    auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds(5);
    // Finish the bounded SDK request before returning; its block owns the image.
    while (dispatch_semaphore_wait(ready, dispatch_time(DISPATCH_TIME_NOW, 20 * NSEC_PER_MSEC)) !=
           0) {
        check();
        require(std::chrono::steady_clock::now() < deadline, "ScreenCaptureKit capture timed out");
    }
    CGImageRef image = (__bridge CGImageRef)capturedImage;
    require(image && !error, "ScreenCaptureKit screenshot failed");
    check();
    require(sameRect(desktop, displaySource ? CGDisplayBounds(displayID(identity)) : windowBounds(lookup(identity))),
            "Source geometry changed during capture; observe again");
    desktop = sourceDesktop;
    size_t width = CGImageGetWidth(image), height = CGImageGetHeight(image);
    NSMutableData *data = [NSMutableData data];
    CGImageDestinationRef encoder = CGImageDestinationCreateWithData(
        (__bridge CFMutableDataRef)data, CFSTR("public.png"), 1, nullptr);
    require(encoder, "PNG encoder unavailable");
    CGImageDestinationAddImage(encoder, image, nullptr);
    bool encoded = CGImageDestinationFinalize(encoder);
    CFRelease(encoder);
    require(encoded && data.length <= 20 * 1024 * 1024, "PNG encoding failed or exceeds limit");
    if (!enrich) {
        return @{
            @"png" : [data base64EncodedStringWithOptions:0],
            @"transform" : @{@"desktop" : rect(desktop), @"width" : @(width), @"height" : @(height)},
            @"elements" : @[], @"accessibility_status" : @"preview only", @"ocr_status" : @"preview only"
        };
    }
    NSMutableArray *elements = [NSMutableArray array];
    NSString *axStatus = @"AX selected-window labels/roles/bounds; at most 256 nodes, depth 12, "
                         @"750 ms; protected values omitted";
    @try {
        try {
            if (displaySource)
                axStatus = @"unavailable: display observations use visible pixels and OCR; window-only AX targets may be occluded";
            else
                accessibility(w, desktop, width, height, elements);
        } catch (const std::exception &e) {
            axStatus = [@"unavailable: " stringByAppendingString:@(e.what())];
        }
    } @catch (NSException *e) {
        axStatus = [@"unavailable: " stringByAppendingString:e.reason ?: @"AX exception"];
    }
    NSString *ocrStatus = @"Vision local text recognition; bounded to 256 regions; OCR is not "
                          @"evidence of interactivity";
    VNRecognizeTextRequest *request = [VNRecognizeTextRequest new];
    request.recognitionLevel = VNRequestTextRecognitionLevelFast;
    request.usesLanguageCorrection = NO;
    // Vision is cancellable from its progress callback and uses the exact image.
    auto ocrDeadline = std::chrono::steady_clock::now() + std::chrono::seconds(2);
    request.progressHandler = ^(VNRequest *r, double progress, NSError *e) {
      (void)progress;
      (void)e;
      if (cancelled.load() || std::chrono::steady_clock::now() > ocrDeadline)
          [r cancel];
    };
    __block NSError *ocrError = nil;
    __block BOOL recognized = NO;
    VNImageRequestHandler *handler = [[VNImageRequestHandler alloc] initWithCGImage:image
                                                                            options:@{}];
    dispatch_semaphore_t ocrReady = dispatch_semaphore_create(0);
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_UTILITY, 0), ^{
      @try {
          recognized = [handler performRequests:@[ request ] error:&ocrError];
      } @catch (NSException *e) {
          ocrError = [NSError
              errorWithDomain:@"KomaVision"
                         code:1
                     userInfo:@{NSLocalizedDescriptionKey : e.reason ?: @"Vision exception"}];
      }
      dispatch_semaphore_signal(ocrReady);
    });
    bool ocrFinished = false;
    while (std::chrono::steady_clock::now() < ocrDeadline && !cancelled.load()) {
        if (dispatch_semaphore_wait(ocrReady,
                                    dispatch_time(DISPATCH_TIME_NOW, 20 * NSEC_PER_MSEC)) == 0) {
            ocrFinished = true;
            break;
        }
    }
    if (!ocrFinished) {
        [request cancel];
        ocrStatus = @"unavailable: Vision OCR timed out or was cancelled";
    } else if (recognized) {
        unsigned count = 0;
        for (VNRecognizedTextObservation *o in request.results) {
            if (count++ >= 256)
                break;
            VNRecognizedText *text = [o topCandidates:1].firstObject;
            if (!text)
                continue;
            CGRect b = o.boundingBox;
            CGRect px = CGRectMake(b.origin.x * width, (1 - b.origin.y - b.size.height) * height,
                                   b.size.width * width, b.size.height * height);
            [elements addObject:@{
                @"id" : @"",
                @"source" : @"ocr",
                @"label" : limited(text.string),
                @"role" : @"text",
                @"bounds" : rect(px),
                @"enabled" : @NO,
                @"selected" : @NO,
                @"focused" : @NO,
                @"confidence" : @(text.confidence)
            }];
        }
    } else
        ocrStatus = [@"unavailable: "
            stringByAppendingString:ocrError.localizedDescription ?: @"Vision failed"];
    return @{
        @"png" : [data base64EncodedStringWithOptions:0],
        @"transform" : @{@"desktop" : rect(desktop), @"width" : @(width), @"height" : @(height)},
        @"elements" : elements,
        @"accessibility_status" : axStatus,
        @"ocr_status" : ocrStatus
    };
}
static id dispatch(NSDictionary *r) {
    NSString *command = r[@"command"];
    if ([command isEqual:@"release"]) {
        release();
        return NSNull.null;
    }
    if ([command isEqual:@"reset"]) {
        release();
        cancelled = false;
        target = nil;
        return NSNull.null;
    }
    if ([command isEqual:@"capabilities"]) {
        bool supported = false;
        if (@available(macOS 14.0, *))
            supported = true;
        if (supported && [r[@"prompt"] boolValue]) {
            CGRequestScreenCaptureAccess();
            AXIsProcessTrustedWithOptions(
                (__bridge CFDictionaryRef)
                    @{(__bridge NSString *)kAXTrustedCheckOptionPrompt : @YES});
        }
        bool capture = supported && CGPreflightScreenCaptureAccess(), ax = AXIsProcessTrusted();
        NSMutableArray *limits = [NSMutableArray array];
        if (!supported)
            [limits addObject:@"Computer observation requires macOS 14 or later"];
        if (!capture)
            [limits addObject:@"Grant Screen Recording access in System Settings, then reactivate"];
        if (!ax)
            [limits addObject:@"Grant Accessibility access in System Settings for focus/input and "
                              @"labels, then reactivate"];
        [limits addObject:@"Screen shares include visible windows, dialogs and desktop chrome. An application share is that window only: input brings it forward and the real pointer glides to the point. An enlarged arrow follows that pointer during the glide and is left out of the screenshot. AX metadata is available for application observations; clicks use screenshot coordinates."];
        return @{
            @"capture" : @(capture),
            @"windows" : @(capture),
            @"focus" : @(capture),
            @"pointer" : @(capture && ax),
            @"keyboard" : @(capture && ax),
            @"accessibility" : @(capture && ax),
            @"ocr" : @(supported),
            @"floating" : @YES,
            @"limitations" : limits
        };
    }
    check();
    if ([command isEqual:@"windows"]) {
        NSMutableArray *result = [NSMutableArray array];
        CGDirectDisplayID displays[32];
        uint32_t count = 0;
        require(CGGetActiveDisplayList(32, displays, &count) == kCGErrorSuccess, "Display discovery failed");
        for (uint32_t i = 0; i < count; ++i) {
            check();
            [result addObject:describeDisplay(displays[i])];
        }
        if (@available(macOS 14.0, *)) {
            SCShareableContent *content = shareableContent();
            for (SCWindow *w in content.windows) {
                check();
                if (result.count >= 288) break;
                SCRunningApplication *app = w.owningApplication;
                CGRect frame = w.frame;
                if (!app || app.processID <= 0 || app.processID == getpid() ||
                    w.windowLayer != 0 || CGRectIsEmpty(frame) || CGRectIsNull(frame)) continue;
                NSString *application = limited(app.applicationName);
                if (!application.length) application = limited(app.bundleIdentifier);
                if (!application.length) application = [NSString stringWithFormat:@"Application %d", app.processID];
                NSString *title = limited(w.title);
                [result addObject:@{
                    @"id": [NSString stringWithFormat:@"%u:%d", w.windowID, app.processID],
                    @"application": application,
                    @"title": title.length ? title : application,
                    @"geometry": rect(frame),
                    @"focused": @NO
                }];
            }
        }
        return result;
    }
    if ([command isEqual:@"inspect"] || [command isEqual:@"select"]) {
        if (isDisplay(r[@"window"])) {
            target = describeDisplay(displayID(r[@"window"]));
            return target;
        }
        NSDictionary *w = lookup(r[@"window"]);
        target = describe(w);
        return target;
    }
    if ([command isEqual:@"capture"] || [command isEqual:@"preview"]) {
        if (@available(macOS 14.0, *))
            return capture(r[@"window"], [command isEqual:@"capture"], r[@"region"]);
        throw std::runtime_error("Capture requires macOS 14+");
    }
    if ([command isEqual:@"validate_input"]) {
        guardInput();
        NSString *kind = r[@"action"][@"kind"];
        if ([kind isEqual:@"type"] || [kind isEqual:@"key"]) guardKeyboard();
        if ([kind isEqual:@"key"])
            for (NSString *name in r[@"action"][@"keys"]) keyCode(name);
        return NSNull.null;
    }
    if ([command isEqual:@"pointer"]) {
        CGPoint point = currentPointer();
        return @{@"x" : @(point.x), @"y" : @(point.y)};
    }
    if ([command isEqual:@"input"]) {
        try {
            input(r[@"action"], r[@"transform"], r[@"glide"][@"steps"]);
        } catch (...) {
            release();
            throw;
        }
        return NSNull.null;
    }
    throw std::runtime_error("Unknown native desktop command");
}
extern "C" char *koma_computer_call(const char *json) {
    @autoreleasepool {
        inputStarted = false;
        NSDictionary *reply;
        @try {
            try {
                NSData *data = [NSData dataWithBytes:json length:strlen(json)];
                NSDictionary *r = [NSJSONSerialization JSONObjectWithData:data options:0 error:nil];
                require([r isKindOfClass:NSDictionary.class], "Invalid native request");
                reply = @{@"result" : dispatch(r)};
            } catch (const InputBusy &) {
                reply = @{@"error" : @"Keyboard or mouse is busy", @"input_busy" : @YES,
                          @"input_started" : @(inputStarted)};
            } catch (const std::exception &e) {
                reply = @{@"error" : @(e.what())};
            }
        } @catch (NSException *e) {
            reply = @{@"error" : e.reason ?: @"Native SDK exception"};
        }
        NSData *data = [NSJSONSerialization dataWithJSONObject:reply options:0 error:nil];
        NSString *text = [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding];
        return strdup(text.UTF8String ?: "{\"error\":\"Native serialization failed\"}");
    }
}
extern "C" void koma_computer_free(char *reply) { free(reply); }
extern "C" void koma_computer_cancel() { cancelled = true; }

// The bundle icon only applies when Launch Services starts this process as
// Koma.app. `koma gui` from a terminal is a bare executable, so the Dock and
// menu bar stay on the generic icon until the image is set explicitly.
extern "C" void koma_set_app_icon(const char *path) {
    if (path == nullptr || path[0] == '\0') {
        return;
    }
    @autoreleasepool {
        NSString *file = [NSString stringWithUTF8String:path];
        if (file == nil) {
            return;
        }
        NSImage *image = [[NSImage alloc] initWithContentsOfFile:file];
        if (image != nil) {
            [[NSApplication sharedApplication] setApplicationIconImage:image];
        }
    }
}

// A frameless window can stay behind whatever macOS put up during launch.
// Both calls predate macOS 11. activateIgnoringOtherApps: is deprecated in
// the 14 SDK; its replacement is not on the 11.0 deployment target.
extern "C" void koma_activate_app(void) {
    @autoreleasepool {
        NSApplication *app = [NSApplication sharedApplication];
        [app setActivationPolicy:NSApplicationActivationPolicyRegular];
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
        [app activateIgnoringOtherApps:YES];
#pragma clang diagnostic pop
    }
}

// Menu-bar usage card. Field order is the Rust `KomaUsageStats` ABI (`repr(C)`).
// Do not use `@available` or NSProcessInfo in this file: `@available` lowers to
// ___isPlatformVersionAtLeast, which this object cannot link.
struct KomaUsageStats {
    uint64_t mem_window;
    uint64_t mem_agent;
    uint64_t mem_services;
    uint64_t mem_system;
    uint64_t tokens_in;
    uint64_t tokens_cached;
    uint64_t tokens_out;
    uint64_t cost_micros;
    uint64_t context_window;
    uint8_t working;
};

static bool path_is_koma_or_webkit(const char *path) {
    return path != nullptr && (std::strstr(path, "koma") != nullptr || std::strstr(path, "Koma") != nullptr ||
                                std::strstr(path, "WebKit") != nullptr || std::strstr(path, "webkit") != nullptr);
}

// Resident bytes for a live pid. Zero when the pid is dead or its executable
// is neither this process, Koma, nor a WebKit helper (pid reuse guard).
extern "C" uint64_t koma_resident_size(uint32_t pid) {
    if (pid == 0)
        return 0;
    char path[PROC_PIDPATHINFO_MAXSIZE];
    std::memset(path, 0, sizeof(path));
    int n = proc_pidpath((pid_t)pid, path, sizeof(path));
    if (n <= 0)
        return 0;
    path[sizeof(path) - 1] = '\0';
    if (pid != (uint32_t)getpid() && !path_is_koma_or_webkit(path))
        return 0;
    struct proc_taskinfo info;
    std::memset(&info, 0, sizeof(info));
    int wrote = proc_pidinfo((pid_t)pid, PROC_PIDTASKINFO, 0, &info, (int)sizeof(info));
    if (wrote != (int)sizeof(info))
        return 0;
    return info.pti_resident_size;
}

extern "C" uint64_t koma_physical_memory(void) {
    uint64_t mem = 0;
    size_t len = sizeof(mem);
    if (sysctlbyname("hw.memsize", &mem, &len, nullptr, 0) != 0)
        return 0;
    return mem;
}

// Child pids of `pid`, capped by `cap`. The return is how many were written.
extern "C" uint32_t koma_child_pids(uint32_t pid, uint32_t *out, uint32_t cap) {
    if (out == nullptr || cap == 0 || pid == 0)
        return 0;
    pid_t storage[256];
    uint32_t slots = 64;
    int n = proc_listchildpids((pid_t)pid, storage, slots * sizeof(pid_t));
    if (n == (int)slots) {
        slots = 256;
        n = proc_listchildpids((pid_t)pid, storage, slots * sizeof(pid_t));
    }
    if (n <= 0)
        return 0;
    if (n > (int)slots)
        n = (int)slots;
    uint32_t count = 0;
    for (int i = 0; i < n && count < cap; ++i) {
        if (storage[i] > 0)
            out[count++] = (uint32_t)storage[i];
    }
    return count;
}

static NSString *fmt_bytes(uint64_t bytes) {
    double mb = (double)bytes / (1024.0 * 1024.0);
    if (mb >= 1024.0) {
        double gb = mb / 1024.0;
        if (gb >= 10.0)
            return [NSString stringWithFormat:@"%.0f GB", gb];
        return [NSString stringWithFormat:@"%.1f GB", gb];
    }
    if (mb >= 100.0)
        return [NSString stringWithFormat:@"%.0f MB", mb];
    return [NSString stringWithFormat:@"%.1f MB", mb];
}

static NSString *fmt_center_bytes(uint64_t bytes) {
    double mb = (double)bytes / (1024.0 * 1024.0);
    if (mb >= 1024.0)
        return [NSString stringWithFormat:@"%.1fG", mb / 1024.0];
    if (mb >= 10.0)
        return [NSString stringWithFormat:@"%.0fM", mb];
    return [NSString stringWithFormat:@"%.1fM", mb];
}

static NSString *fmt_tokens(uint64_t n) {
    if (n >= 10000ull) {
        NSString *raw = [NSString stringWithFormat:@"%.1f", (double)n / 1000.0];
        if ([raw hasSuffix:@".0"])
            raw = [raw substringToIndex:raw.length - 2];
        return [raw stringByAppendingString:@"k"];
    }
    return [NSString stringWithFormat:@"%llu", (unsigned long long)n];
}

static NSString *fmt_cost(uint64_t micros) {
    return [NSString stringWithFormat:@"$%.4f", (double)micros / 1000000.0];
}

static unsigned token_percent(const KomaUsageStats *stats) {
    if (stats->context_window == 0)
        return 0;
    double pct = (double)stats->tokens_in * 100.0 / (double)stats->context_window;
    if (pct < 0)
        return 0;
    if (pct > 999)
        return 999;
    return (unsigned)(pct + 0.5);
}

static NSString *menu_title(const KomaUsageStats *stats) {
    uint64_t total = stats->mem_window + stats->mem_agent + stats->mem_services;
    NSString *title = @"Koma";
    if (total > 0 && stats->context_window > 0) {
        title = [NSString stringWithFormat:@"%@ · %u%%", fmt_bytes(total), token_percent(stats)];
    } else if (total > 0) {
        title = fmt_bytes(total);
    } else if (stats->context_window > 0) {
        title = [NSString stringWithFormat:@"%u%%", token_percent(stats)];
    }
    if (stats->working)
        title = [@"● " stringByAppendingString:title];
    return title;
}

static void draw_text(NSString *text, NSRect rect, NSFont *font, NSColor *color, NSTextAlignment align) {
    if (text == nil)
        return;
    NSMutableParagraphStyle *style = [[NSMutableParagraphStyle alloc] init];
    style.alignment = align;
    style.lineBreakMode = NSLineBreakByTruncatingTail;
    [text drawInRect:rect withAttributes:@{
        NSFontAttributeName : font,
        NSForegroundColorAttributeName : color,
        NSParagraphStyleAttributeName : style,
    }];
}

static void draw_swatch(NSRect rect, NSColor *color) {
    if (color == nil)
        return;
    NSBezierPath *path = [NSBezierPath bezierPathWithRoundedRect:rect xRadius:1.5 yRadius:1.5];
    [color setFill];
    [path fill];
}

// `fractions` are 0..1 shares of the circle, drawn clockwise from 12 o'clock.
static void draw_ring(NSPoint center, CGFloat radius, CGFloat width, const CGFloat *fractions, NSColor *const *colors,
                      int count) {
    NSBezierPath *track = [NSBezierPath bezierPath];
    [track appendBezierPathWithArcWithCenter:center radius:radius startAngle:90 endAngle:90 - 360 clockwise:YES];
    track.lineWidth = width;
    [[NSColor colorWithSRGBRed:1 green:1 blue:1 alpha:0.14] setStroke];
    [track stroke];
    CGFloat angle = 90;
    for (int i = 0; i < count; ++i) {
        if (fractions[i] <= 0 || colors[i] == nil)
            continue;
        CGFloat sweep = fractions[i] * 360.0;
        if (sweep > 360)
            sweep = 360;
        NSBezierPath *arc = [NSBezierPath bezierPath];
        [arc appendBezierPathWithArcWithCenter:center
                                        radius:radius
                                    startAngle:angle
                                      endAngle:angle - sweep
                                     clockwise:YES];
        arc.lineWidth = width;
        [colors[i] setStroke];
        [arc stroke];
        angle -= sweep;
    }
}

static void draw_legend(CGFloat y, NSColor *swatch, NSString *label, NSString *value, NSFont *font, NSColor *dim,
                        NSColor *fg) {
    draw_swatch(NSMakeRect(108, y + 3, 8, 8), swatch);
    draw_text(label, NSMakeRect(122, y, 80, 14), font, dim, NSTextAlignmentLeft);
    draw_text(value, NSMakeRect(188, y, 98, 14), font, fg, NSTextAlignmentRight);
}

static void draw_center(NSString *text, NSPoint center, NSFont *font, NSColor *color) {
    NSDictionary *attrs = @{NSFontAttributeName : font, NSForegroundColorAttributeName : color};
    NSSize size = [text sizeWithAttributes:attrs];
    [text drawAtPoint:NSMakePoint(center.x - size.width / 2.0, center.y - size.height / 2.0) withAttributes:attrs];
}

static const CGFloat kCardW = 300;
static const CGFloat kCardH = 288;

// Five role rows sit under the token block. `roles` is 0..5.
static CGFloat card_height(uint32_t roles) {
    if (roles == 0)
        return kCardH;
    if (roles > 5)
        roles = 5;
    return 310 + (CGFloat)(roles - 1) * 16 + 14 + 12;
}

static NSString *ns_utf8(const char *text) {
    if (text == nullptr || text[0] == '\0')
        return @"—";
    NSString *value = [NSString stringWithUTF8String:text];
    return value != nil ? value : @"—";
}

static void draw_role(CGFloat y, NSString *label, NSString *value, NSFont *font, NSColor *dim, NSColor *fg) {
    draw_text(label, NSMakeRect(14, y, 78, 14), font, dim, NSTextAlignmentLeft);
    draw_text(value, NSMakeRect(96, y, 190, 14), font, fg, NSTextAlignmentRight);
}

@interface KomaUsageView : NSView
- (void)setStats:(KomaUsageStats)stats;
- (void)setRoles:(const char *const *)labels values:(const char *const *)values count:(uint32_t)count;
- (CGFloat)cardHeight;
@end

@implementation KomaUsageView {
    KomaUsageStats _stats;
    NSMutableArray<NSString *> *_roleLabels;
    NSMutableArray<NSString *> *_roleValues;
}

- (void)setStats:(KomaUsageStats)stats {
    _stats = stats;
    [self setNeedsDisplay:YES];
}

- (void)setRoles:(const char *const *)labels values:(const char *const *)values count:(uint32_t)count {
    if (labels == nullptr || values == nullptr)
        count = 0;
    if (count > 5)
        count = 5;
    _roleLabels = [NSMutableArray arrayWithCapacity:count];
    _roleValues = [NSMutableArray arrayWithCapacity:count];
    for (uint32_t i = 0; i < count; ++i) {
        [_roleLabels addObject:ns_utf8(labels[i])];
        [_roleValues addObject:ns_utf8(values[i])];
    }
    [self setNeedsDisplay:YES];
}

- (CGFloat)cardHeight {
    return card_height((uint32_t)_roleLabels.count);
}

- (void)drawRect:(NSRect)dirty {
    (void)dirty;
    NSRect bounds = self.bounds;
    [[NSColor colorWithSRGBRed:0.11 green:0.11 blue:0.125 alpha:1] setFill];
    NSRectFill(bounds);
    CGFloat H = bounds.size.height;
    NSColor *fg = [NSColor colorWithSRGBRed:0.96 green:0.96 blue:0.97 alpha:1];
    NSColor *dim = [NSColor colorWithSRGBRed:0.70 green:0.71 blue:0.76 alpha:1];
    NSColor *windowColor = [NSColor colorWithSRGBRed:0.30 green:0.55 blue:1 alpha:1];
    NSColor *agentColor = [NSColor colorWithSRGBRed:1 green:0.36 blue:0.36 alpha:1];
    NSColor *serviceColor = [NSColor colorWithSRGBRed:0.75 green:0.52 blue:0.99 alpha:1];
    NSColor *tokenColor = [NSColor colorWithSRGBRed:0.24 green:0.86 blue:0.55 alpha:1];
    NSFont *titleFont = [NSFont systemFontOfSize:13 weight:NSFontWeightSemibold];
    NSFont *sectionFont = [NSFont systemFontOfSize:12 weight:NSFontWeightSemibold];
    NSFont *rowFont = [NSFont monospacedDigitSystemFontOfSize:11 weight:NSFontWeightRegular];
    NSFont *centerFont = [NSFont monospacedDigitSystemFontOfSize:11 weight:NSFontWeightSemibold];

    auto rowY = [&](CGFloat top) { return H - top - 14; };
    draw_text(@"Koma", NSMakeRect(14, rowY(12), 180, 16), titleFont, fg, NSTextAlignmentLeft);
    if (_stats.working) {
        NSRect dot = NSMakeRect(bounds.size.width - 22, rowY(12) + 4, 7, 7);
        [tokenColor setFill];
        [[NSBezierPath bezierPathWithOvalInRect:dot] fill];
    }

    draw_text(@"Memory", NSMakeRect(14, rowY(36), 80, 16), sectionFont, fg, NSTextAlignmentLeft);
    uint64_t total = _stats.mem_window + _stats.mem_agent + _stats.mem_services;
    CGFloat memFrac[3] = {0, 0, 0};
    NSColor *memColor[3] = {windowColor, agentColor, serviceColor};
    if (total > 0) {
        memFrac[0] = (CGFloat)_stats.mem_window / (CGFloat)total;
        memFrac[1] = (CGFloat)_stats.mem_agent / (CGFloat)total;
        memFrac[2] = (CGFloat)_stats.mem_services / (CGFloat)total;
    }
    NSPoint memCenter = NSMakePoint(48, rowY(104));
    draw_ring(memCenter, 26, 7, memFrac, memColor, 3);
    draw_center(total > 0 ? fmt_center_bytes(total) : @"0", memCenter, centerFont, fg);
    draw_legend(rowY(58), windowColor, @"Window", fmt_bytes(_stats.mem_window), rowFont, dim, fg);
    draw_legend(rowY(76), agentColor, @"Agent", fmt_bytes(_stats.mem_agent), rowFont, dim, fg);
    draw_legend(rowY(94), serviceColor, @"Services", fmt_bytes(_stats.mem_services), rowFont, dim, fg);
    draw_legend(rowY(112), nil, @"Total", fmt_bytes(total), rowFont, dim, fg);
    if (_stats.mem_system > 0) {
        NSString *of = [NSString stringWithFormat:@"of %@ on this machine", fmt_bytes(_stats.mem_system)];
        draw_text(of, NSMakeRect(122, rowY(130), 164, 14), rowFont, dim, NSTextAlignmentLeft);
    }

    [[NSColor colorWithSRGBRed:1 green:1 blue:1 alpha:0.08] setFill];
    NSRectFill(NSMakeRect(14, rowY(148), bounds.size.width - 28, 1));

    draw_text(@"Tokens", NSMakeRect(14, rowY(162), 80, 16), sectionFont, fg, NSTextAlignmentLeft);
    CGFloat tokenFrac = 0;
    NSString *pct = @"—";
    if (_stats.context_window > 0) {
        unsigned shown = token_percent(&_stats);
        pct = [NSString stringWithFormat:@"%u%%", shown];
        tokenFrac = (CGFloat)shown / 100.0;
        if (tokenFrac > 1)
            tokenFrac = 1;
    }
    CGFloat one = tokenFrac;
    NSColor *oneColor = tokenColor;
    NSPoint tokenCenter = NSMakePoint(48, rowY(230));
    draw_ring(tokenCenter, 26, 7, &one, &oneColor, 1);
    draw_center(pct, tokenCenter, centerFont, fg);
    draw_legend(rowY(184), tokenColor, @"Context", pct, rowFont, dim, fg);
    draw_legend(rowY(202), nil, @"In", fmt_tokens(_stats.tokens_in), rowFont, dim, fg);
    draw_legend(rowY(220), nil, @"Cached", fmt_tokens(_stats.tokens_cached), rowFont, dim, fg);
    draw_legend(rowY(238), nil, @"Out", fmt_tokens(_stats.tokens_out), rowFont, dim, fg);
    draw_legend(rowY(256), nil, @"Cost", fmt_cost(_stats.cost_micros), rowFont, dim, fg);

    NSUInteger roleCount = _roleLabels.count;
    if (roleCount > 0 && _roleValues.count == roleCount) {
        [[NSColor colorWithSRGBRed:1 green:1 blue:1 alpha:0.08] setFill];
        NSRectFill(NSMakeRect(14, rowY(274), bounds.size.width - 28, 1));
        draw_text(@"Roles", NSMakeRect(14, rowY(288), 80, 16), sectionFont, fg, NSTextAlignmentLeft);
        for (NSUInteger i = 0; i < roleCount; ++i) {
            draw_role(rowY(310 + (CGFloat)i * 16), _roleLabels[i], _roleValues[i], rowFont, dim, fg);
        }
    }
}

@end

@interface KomaUsageBar : NSObject <NSPopoverDelegate>
- (void)install;
- (void)apply:(const KomaUsageStats *)stats;
- (void)setRoles:(const char *const *)labels values:(const char *const *)values count:(uint32_t)count;
@end

@implementation KomaUsageBar {
    NSStatusItem *_item;
    NSPopover *_popover;
    KomaUsageView *_view;
    BOOL _suppressToggle;
}

- (void)install {
    if (_item != nil)
        return;
    _item = [[NSStatusBar systemStatusBar] statusItemWithLength:NSVariableStatusItemLength];
    NSStatusBarButton *button = _item.button;
    button.title = @"Koma";
    button.target = self;
    button.action = @selector(toggle:);
    button.toolTip = @"Koma memory, tokens, and models";
    button.font = [NSFont monospacedDigitSystemFontOfSize:12 weight:NSFontWeightMedium];

    _view = [[KomaUsageView alloc] initWithFrame:NSMakeRect(0, 0, kCardW, kCardH)];
    NSViewController *controller = [[NSViewController alloc] init];
    controller.view = _view;
    _popover = [[NSPopover alloc] init];
    _popover.behavior = NSPopoverBehaviorTransient;
    _popover.animates = YES;
    _popover.appearance = [NSAppearance appearanceNamed:NSAppearanceNameDarkAqua];
    _popover.contentViewController = controller;
    _popover.contentSize = NSMakeSize(kCardW, kCardH);
    _popover.delegate = self;
}

- (void)apply:(const KomaUsageStats *)stats {
    if (_view == nil || stats == nullptr)
        return;
    [_view setStats:*stats];
    NSStatusBarButton *button = _item.button;
    if (button != nil)
        button.title = menu_title(stats);
}

- (void)setRoles:(const char *const *)labels values:(const char *const *)values count:(uint32_t)count {
    if (_view == nil)
        return;
    [_view setRoles:labels values:values count:count];
    CGFloat height = [_view cardHeight];
    [_view setFrameSize:NSMakeSize(kCardW, height)];
    if (_popover != nil)
        _popover.contentSize = NSMakeSize(kCardW, height);
}

- (void)toggle:(id)sender {
    if (_suppressToggle) {
        _suppressToggle = NO;
        return;
    }
    if (_popover.shown) {
        [_popover performClose:sender];
        return;
    }
    NSStatusBarButton *button = _item.button;
    if (button == nil)
        return;
    [_popover showRelativeToRect:button.bounds ofView:button preferredEdge:NSRectEdgeMinY];
}

- (void)popoverDidClose:(NSNotification *)notification {
    (void)notification;
    // A click on the status item both closes a transient popover and fires
    // the button action. Ignore that action or the card reopens immediately.
    _suppressToggle = YES;
    dispatch_async(dispatch_get_main_queue(), ^{
        self->_suppressToggle = NO;
    });
}

@end

static KomaUsageBar *sharedUsageBar = nil;

extern "C" void koma_usage_bar_update(const KomaUsageStats *stats) {
    if (stats == nullptr)
        return;
    @autoreleasepool {
        if (sharedUsageBar == nil) {
            sharedUsageBar = [KomaUsageBar new];
            [sharedUsageBar install];
        }
        [sharedUsageBar apply:stats];
    }
}

extern "C" void koma_usage_bar_set_roles(const char *const *labels, const char *const *values, uint32_t count) {
    @autoreleasepool {
        if (sharedUsageBar == nil) {
            sharedUsageBar = [KomaUsageBar new];
            [sharedUsageBar install];
        }
        [sharedUsageBar setRoles:labels values:values count:count];
    }
}
