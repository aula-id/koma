// macOS GUI-only SDK bridge. Captures serve model observations or ephemeral live
// preview frames. Only model observations run enrichment. Desktop strings are data.
#import <AppKit/AppKit.h>
#import <ApplicationServices/ApplicationServices.h>
#import <ImageIO/ImageIO.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>
#import <Vision/Vision.h>
#include <algorithm>
#include <atomic>
#include <chrono>
#include <cmath>
#include <stdexcept>
#include <thread>
#include <unistd.h>
#include <vector>

static std::atomic<bool> cancelled(false);
static NSDictionary *target;
static std::vector<CGKeyCode> held;
static CGEventFlags flags = 0;
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
    return NSWorkspace.sharedWorkspace.frontmostApplication.processIdentifier ==
               [w[(id)kCGWindowOwnerPID] intValue] &&
           front && sameRect(axBounds((__bridge AXUIElementRef)front), windowBounds(w));
}
static NSDictionary *describe(NSDictionary *w) {
    return @{
        @"id" :
            [NSString stringWithFormat:@"%@:%@", w[(id)kCGWindowNumber], w[(id)kCGWindowOwnerPID]],
        @"application" : limited(w[(id)kCGWindowOwnerName]),
        @"title" : limited(w[(id)kCGWindowName]),
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
        if ([above[(id)kCGWindowAlpha] doubleValue] <= 0)
            continue;
        CGRect overlap = CGRectIntersection(r, windowBounds(above));
        require(CGRectIsNull(overlap) || CGRectIsEmpty(overlap),
                "Target obstructed by another window; ask the user to clear it before reactivating control");
    }
    require(found, "Target no longer visible");
}
static void guardInput() {
    check();
    require(target != nil, "No input target");
    NSDictionary *w = lookup(target[@"id"]);
    require(sameRect(windowBounds(w), bounds(target[@"geometry"])), "Window geometry changed");
    require([limited(w[(id)kCGWindowName]) isEqual:target[@"title"]],
            "Window title changed; observe again");
    require(focused(w), "Selected window lost focus");
    unobstructed(w);
    for (CGKeyCode code = 0; code < 128; ++code)
        require(!CGEventSourceKeyState(kCGEventSourceStateCombinedSessionState, code) ||
                    std::find(held.begin(), held.end(), code) != held.end(),
                "Release physical keys before computer input");
    for (uint32_t button = 0; button < 5; ++button)
        require(!CGEventSourceButtonState(kCGEventSourceStateCombinedSessionState,
                                          static_cast<CGMouseButton>(button)),
                "Release physical mouse buttons before computer input");
}
static void release() {
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
    require(code != nil, "Unsupported macOS key name");
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
    CGEventPost(kCGHIDEventTap, e);
    CFRelease(e);
}
static CGPoint mapPoint(NSDictionary *a, NSDictionary *t) {
    CGRect r = bounds(t[@"desktop"]);
    double x = [a[@"x"] doubleValue], y = [a[@"y"] doubleValue];
    double width = [t[@"width"] doubleValue], height = [t[@"height"] doubleValue];
    require(x >= 0 && y >= 0 && x < width && y < height, "Invalid screenshot coordinate");
    return CGPointMake(r.origin.x + x * r.size.width / width,
                       r.origin.y + y * r.size.height / height);
}
static void input(NSDictionary *a, NSDictionary *t) {
    guardInput();
    NSString *kind = a[@"kind"];
    if ([kind isEqual:@"type"]) {
        NSString *text = a[@"text"];
        for (NSUInteger i = 0; i < text.length;) {
            guardInput();
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
            CGEventPost(kCGHIDEventTap, down);
            CGEventPost(kCGHIDEventTap, up);
            CFRelease(down);
            CFRelease(up);
            i = NSMaxRange(range);
        }
    } else if ([kind isEqual:@"key"]) {
        std::vector<CGKeyCode> codes;
        for (NSString *s in a[@"keys"])
            codes.push_back(keyCode(s));
        for (auto code : codes)
            key(code, true);
        release();
    } else {
        CGPoint p = mapPoint(a, t);
        CGEventRef move =
            CGEventCreateMouseEvent(nullptr, kCGEventMouseMoved, p, kCGMouseButtonLeft);
        require(move, "Unable to move pointer");
        CGEventPost(kCGHIDEventTap, move);
        CFRelease(move);
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
                CGEventPost(kCGHIDEventTap, down);
                CGEventPost(kCGHIDEventTap, up);
                CFRelease(down);
                CFRelease(up);
            }
        } else if ([kind isEqual:@"scroll"]) {
            guardInput();
            CGEventRef e = CGEventCreateScrollWheelEvent(nullptr, kCGScrollEventUnitLine, 1,
                                                         -[a[@"delta"] intValue]);
            require(e, "Unable to scroll");
            CGEventPost(kCGHIDEventTap, e);
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
static NSDictionary *capture(NSString *identity, bool enrich) API_AVAILABLE(macos(14.0)) {
    check();
    NSDictionary *w = lookup(identity);
    CGRect desktop = windowBounds(w);
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
    while (dispatch_semaphore_wait(ready, dispatch_time(DISPATCH_TIME_NOW, 20 * NSEC_PER_MSEC)) !=
           0) {
        check();
        require(std::chrono::steady_clock::now() < deadline, "Window discovery timed out");
    }
    require(!error && content, "ScreenCaptureKit window discovery failed");
    SCWindow *selected = nil;
    for (SCWindow *candidate in content.windows)
        if (candidate.windowID == [w[(id)kCGWindowNumber] unsignedIntValue])
            selected = candidate;
    require(selected, "Selected window no longer shareable");
    SCContentFilter *filter = [[SCContentFilter alloc] initWithDesktopIndependentWindow:selected];
    SCStreamConfiguration *configuration = [SCStreamConfiguration new];
    configuration.width = (size_t)llround(filter.contentRect.size.width * filter.pointPixelScale);
    configuration.height = (size_t)llround(filter.contentRect.size.height * filter.pointPixelScale);
    require(configuration.width > 0 && configuration.height > 0 &&
                configuration.width * configuration.height <= 32000000,
            "Capture dimensions exceed limit");
    configuration.showsCursor = !enrich;
    configuration.ignoreShadowsSingleWindow = YES;
    configuration.shouldBeOpaque = YES;
    __block id capturedImage = nil;
    error = nil;
    ready = dispatch_semaphore_create(0);
    [SCScreenshotManager captureImageWithFilter:filter
                                  configuration:configuration
                              completionHandler:^(CGImageRef img, NSError *e) {
                                if (img)
                                    capturedImage = CFBridgingRelease(CGImageRetain(img));
                                error = e;
                                dispatch_semaphore_signal(ready);
                              }];
    deadline = std::chrono::steady_clock::now() + std::chrono::seconds(5);
    // Finish the bounded SDK request before returning; its block owns the image.
    while (dispatch_semaphore_wait(ready, dispatch_time(DISPATCH_TIME_NOW, 20 * NSEC_PER_MSEC)) !=
           0) {
        check();
        require(std::chrono::steady_clock::now() < deadline, "ScreenCaptureKit capture timed out");
    }
    CGImageRef image = (__bridge CGImageRef)capturedImage;
    require(image && !error, "ScreenCaptureKit screenshot failed");
    check();
    require(sameRect(desktop, windowBounds(lookup(identity))),
            "Window geometry changed during capture");
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
        return @{
            @"capture" : @(capture),
            @"windows" : @(capture),
            @"focus" : @(capture && ax),
            @"pointer" : @(capture && ax),
            @"keyboard" : @(capture && ax),
            @"accessibility" : @(ax),
            @"ocr" : @(supported),
            @"floating" : @YES,
            @"limitations" : limits
        };
    }
    check();
    if ([command isEqual:@"windows"]) {
        NSMutableArray *result = [NSMutableArray array];
        for (NSDictionary *w in windowInfo()) {
            check();
            if (result.count >= 256)
                break;
            if ([w[(id)kCGWindowOwnerPID] intValue] == getpid() ||
                [w[(id)kCGWindowLayer] intValue] != 0)
                continue;
            try {
                [result addObject:describe(w)];
            } catch (...) {
            }
        }
        return result;
    }
    if ([command isEqual:@"inspect"] || [command isEqual:@"select"]) {
        NSDictionary *w = lookup(r[@"window"]);
        if ([command isEqual:@"select"]) {
            AXUIElementRef ax = axWindow(w);
            AXError err = AXUIElementPerformAction(ax, kAXRaiseAction);
            CFRelease(ax);
            require(err == kAXErrorSuccess, "Application refused window focus");
            NSRunningApplication *app = [NSRunningApplication
                runningApplicationWithProcessIdentifier:[w[(id)kCGWindowOwnerPID] intValue]];
            dispatch_sync(dispatch_get_main_queue(), ^{
              // Default activation options; ignoringOtherApps has no effect on macOS 14+.
              [app activateWithOptions:0];
            });
            for (int i = 0; i < 50 && !focused(w); ++i) {
                check();
                std::this_thread::sleep_for(std::chrono::milliseconds(10));
            }
            require(focused(w), "Application did not grant focus");
        }
        target = describe(w);
        return target;
    }
    if ([command isEqual:@"capture"] || [command isEqual:@"preview"]) {
        if (@available(macOS 14.0, *))
            return capture(r[@"window"], [command isEqual:@"capture"]);
        throw std::runtime_error("Capture requires macOS 14+");
    }
    if ([command isEqual:@"input"]) {
        try {
            input(r[@"action"], r[@"transform"]);
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
        NSDictionary *reply;
        @try {
            try {
                NSData *data = [NSData dataWithBytes:json length:strlen(json)];
                NSDictionary *r = [NSJSONSerialization JSONObjectWithData:data options:0 error:nil];
                require([r isKindOfClass:NSDictionary.class], "Invalid native request");
                reply = @{@"result" : dispatch(r)};
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
