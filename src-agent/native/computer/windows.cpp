// Windows 10 1903+ GUI SDK bridge. Graphics Capture opens one session, keeps the
// newest frame from a short poll, and closes immediately. No background capture.
#include <windows.h>
#include "capture_limits.h"
#include "frame_poll.h"
#include "input_idle.h"
#include <UIAutomation.h>
#include <algorithm>
#include <atomic>
#include <chrono>
#include <cstdlib>
#include <cstring>
#include <cwctype>
#include <cwchar>
#include <d3d11.h>
#include <deque>
#include <dwmapi.h>
#include <dxgi.h>
#include <functional>
#include <memory>
#include <string>
#include <thread>
#include <vector>
#include <wincodec.h>
#include <windows.graphics.capture.interop.h>
#include <windows.graphics.directx.direct3d11.interop.h>
#include <winrt/Windows.Data.Json.h>
#include <winrt/Windows.Foundation.Metadata.h>
#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.Graphics.Capture.h>
#include <winrt/Windows.Graphics.DirectX.Direct3D11.h>
#include <winrt/Windows.Graphics.DirectX.h>
using namespace winrt;
using namespace winrt::Windows::Data::Json;
using namespace winrt::Windows::Graphics::Capture;
using namespace winrt::Windows::Graphics::DirectX;
using namespace winrt::Windows::Graphics::DirectX::Direct3D11;
static std::atomic<bool> cancelled(false);
static HWND target = nullptr;
static HMONITOR targetMonitor = nullptr;
static RECT targetRect{};
static std::wstring targetTitle;
static std::wstring targetId;
static std::vector<WORD> held;
static bool inputStarted = false;
static void require(bool condition, const wchar_t *message) {
    if (!condition)
        throw hresult_error(E_FAIL, message);
}
static void check() { require(!cancelled.load(), L"Computer operation cancelled"); }
static bool localSession() {
    DWORD session = 0;
    return !GetSystemMetrics(SM_REMOTESESSION) && !GetSystemMetrics(SM_REMOTECONTROL) &&
           ProcessIdToSessionId(GetCurrentProcessId(), &session) &&
           session == WTSGetActiveConsoleSessionId();
}
struct Finally {
    std::function<void()> f;
    ~Finally() noexcept {
        try {
            f();
        } catch (...) { /* Cleanup must not terminate an in-flight error. */
        }
    }
};
static JsonValue str(std::wstring_view value) {
    return JsonValue::CreateStringValue(hstring(value));
}
static JsonValue num(double value) { return JsonValue::CreateNumberValue(value); }
static JsonValue boolean(bool value) { return JsonValue::CreateBooleanValue(value); }
static JsonObject rectangle(double x, double y, double w, double h) {
    JsonObject r;
    r.SetNamedValue(L"x", num(x));
    r.SetNamedValue(L"y", num(y));
    r.SetNamedValue(L"width", num(w));
    r.SetNamedValue(L"height", num(h));
    return r;
}
static JsonObject rectangle(RECT r) {
    return rectangle(r.left, r.top, r.right - r.left, r.bottom - r.top);
}
static RECT geometry(HWND window) {
    require(IsWindow(window) && IsWindowVisible(window) && !IsIconic(window),
            L"Selected window closed, hidden or minimized");
    RECT r{};
    if (FAILED(DwmGetWindowAttribute(window, DWMWA_EXTENDED_FRAME_BOUNDS, &r, sizeof(r))))
        require(GetWindowRect(window, &r), L"Window bounds unavailable");
    require(r.right > r.left && r.bottom > r.top, L"Invalid window bounds");
    return r;
}
static std::wstring title(HWND window) {
    wchar_t text[513]{};
    GetWindowTextW(window, text, 513);
    std::wstring result(text);
    if (!result.empty() && result.back() >= 0xd800 && result.back() <= 0xdbff)
        result.pop_back();
    return result;
}
static unsigned long long created(DWORD pid) {
    HANDLE process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid);
    require(process, L"Cannot verify window process identity");
    Finally close{[&] { CloseHandle(process); }};
    FILETIME c{}, e{}, k{}, u{};
    require(GetProcessTimes(process, &c, &e, &k, &u), L"Cannot verify process creation time");
    return (static_cast<unsigned long long>(c.dwHighDateTime) << 32) | c.dwLowDateTime;
}
static std::wstring identity(HWND window) {
    DWORD pid = 0;
    GetWindowThreadProcessId(window, &pid);
    return std::to_wstring(reinterpret_cast<uintptr_t>(window)) + L":" + std::to_wstring(pid) +
           L":" + std::to_wstring(created(pid));
}
static HWND lookup(hstring id) {
    std::wstring text = id.c_str();
    size_t a = text.find(L':'), b = text.find(L':', a + 1);
    require(a != text.npos && b != text.npos, L"Invalid window identity");
    HWND window = reinterpret_cast<HWND>(static_cast<uintptr_t>(std::stoull(text.substr(0, a))));
    DWORD pid = 0;
    GetWindowThreadProcessId(window, &pid);
    require(pid && pid != GetCurrentProcessId() && pid == std::stoul(text.substr(a + 1, b - a - 1)),
            L"Window process identity changed");
    require(created(pid) == std::stoull(text.substr(b + 1)), L"Window process was replaced");
    geometry(window);
    return window;
}
static JsonObject describe(HWND window) {
    DWORD pid = 0;
    GetWindowThreadProcessId(window, &pid);
    JsonObject r;
    r.SetNamedValue(L"id", str(identity(window)));
    r.SetNamedValue(L"application", str(L"pid " + std::to_wstring(pid)));
    r.SetNamedValue(L"title", str(title(window)));
    r.SetNamedValue(L"geometry", rectangle(geometry(window)));
    r.SetNamedValue(L"focused", boolean(GetForegroundWindow() == window));
    return r;
}
// Monitor sources capture the composed desktop, including dialogs and panels.
static bool isDisplay(hstring id) { return std::wstring_view(id.c_str()).rfind(L"display:", 0) == 0; }
static MONITORINFOEXW monitorInfo(HMONITOR monitor) {
    MONITORINFOEXW info{};
    info.cbSize = sizeof(info);
    require(GetMonitorInfoW(monitor, reinterpret_cast<MONITORINFO *>(&info)), L"Shared display disconnected; select a display again");
    return info;
}
static std::wstring monitorId(HMONITOR monitor) {
    return L"display:" + std::wstring(monitorInfo(monitor).szDevice);
}
static std::vector<HMONITOR> monitors() {
    std::vector<HMONITOR> result;
    require(EnumDisplayMonitors(nullptr, nullptr, [](HMONITOR monitor, HDC, LPRECT, LPARAM value) -> BOOL {
        reinterpret_cast<std::vector<HMONITOR> *>(value)->push_back(monitor);
        return TRUE;
    }, reinterpret_cast<LPARAM>(&result)), L"Display discovery failed");
    return result;
}
static HMONITOR lookupMonitor(hstring id) {
    for (HMONITOR monitor : monitors()) if (monitorId(monitor) == id.c_str()) return monitor;
    throw hresult_error(E_FAIL, L"Shared display disconnected; select a display again");
}
static std::wstring desktopFocus() {
    HWND front = GetForegroundWindow();
    DWORD pid = 0;
    GetWindowThreadProcessId(front, &pid);
    RECT r{};
    GetWindowRect(front, &r);
    return std::to_wstring(reinterpret_cast<uintptr_t>(front)) + L":" + std::to_wstring(pid) + L":" + title(front)
        + L":" + std::to_wstring(r.left) + L":" + std::to_wstring(r.top)
        + L":" + std::to_wstring(r.right) + L":" + std::to_wstring(r.bottom);
}
static JsonObject describeMonitor(HMONITOR monitor) {
    auto info = monitorInfo(monitor);
    JsonObject r;
    r.SetNamedValue(L"id", str(monitorId(monitor)));
    r.SetNamedValue(L"application", str(L"Desktop"));
    r.SetNamedValue(L"title", str(std::wstring(info.szDevice) + ((info.dwFlags & MONITORINFOF_PRIMARY) ? L" · Main display" : L" · Display")));
    r.SetNamedValue(L"geometry", rectangle(info.rcMonitor));
    r.SetNamedValue(L"focused", boolean(true));
    r.SetNamedValue(L"focus", str(desktopFocus()));
    return r;
}
static void guardKeyboard() {
    if (!targetMonitor) return;
    // Title, bounds, and HWND identity are not a batch gate. A predicted
    // chord or type keeps going when focus moves to another window on this
    // display. A missing foreground window is not a failure: macOS and X11
    // continue in that case too. Only a window whose center has left this
    // display stops the remaining keys.
    HWND front = GetForegroundWindow();
    if (!front) return;
    RECT r{};
    if (!GetWindowRect(front, &r)) return;
    POINT center{r.left + (r.right - r.left) / 2, r.top + (r.bottom - r.top) / 2};
    require(PtInRect(&targetRect, center), L"Keyboard focus is outside the shared display; click a visible window and observe again");
}
static bool same(RECT a, RECT b) {
    return a.left == b.left && a.top == b.top && a.right == b.right && a.bottom == b.bottom;
}
static void unobstructed(HWND window, RECT r) {
    for (HWND above = GetWindow(window, GW_HWNDPREV); above;
         above = GetWindow(above, GW_HWNDPREV)) {
        if (!IsWindowVisible(above) || IsIconic(above))
            continue;
        DWORD cloaked = 0;
        DwmGetWindowAttribute(above, DWMWA_CLOAKED, &cloaked, sizeof(cloaked));
        if (cloaked)
            continue;
        RECT other{};
        if (!GetWindowRect(above, &other))
            continue;
        RECT intersection{};
        require(!IntersectRect(&intersection, &r, &other),
                L"Target obstructed by another window; move the preview");
    }
}
static void desktopAvailable() {
    HDESK desktop = OpenInputDesktop(0, FALSE, DESKTOP_READOBJECTS);
    require(desktop, L"Input desktop unavailable or secure desktop active");
    Finally close{[&] { CloseDesktop(desktop); }};
    wchar_t inputName[256]{}, ownName[256]{};
    DWORD needed = 0;
    require(GetUserObjectInformationW(desktop, UOI_NAME, inputName, sizeof(inputName), &needed) &&
                GetUserObjectInformationW(GetThreadDesktop(GetCurrentThreadId()), UOI_NAME, ownName,
                                          sizeof(ownName), &needed) &&
                std::wcscmp(inputName, ownName) == 0,
            L"Input desktop changed; stop and reactivate control");
}
static void guardInput() {
    check();
    waitForInputIdle(check, [] {
        for (WORD code = 1; code < 255; ++code) {
            if (code == VK_SHIFT || code == VK_CONTROL || code == VK_MENU) continue;
            if ((GetAsyncKeyState(code) & 0x8000) &&
                std::find(held.begin(), held.end(), code) == held.end()) return true;
        }
        return false;
    });
    require(localSession(), L"Computer input requires the local console session");
    require(targetMonitor != nullptr, L"Application sharing is view-only; select a screen before input");
    if (targetMonitor) {
        // Still the same monitor. Screenshot points are mapped onto its
        // current rectangle, so a move or resize does not cancel the batch.
        require(lookupMonitor(hstring(targetId)) == targetMonitor,
                L"Shared display disconnected; select a display again");
    } else {
        require(target && IsWindow(target), L"No live input target");
        require(lookup(hstring(targetId)) == target, L"Input window identity changed");
        require(same(geometry(target), targetRect) && title(target) == targetTitle, L"Window geometry or title changed; observe again");
        require(GetForegroundWindow() == target, L"Selected window lost focus");
        unobstructed(target, targetRect);
    }
    desktopAvailable();
}
static DWORD extended(WORD code) {
    return code == VK_RCONTROL || code == VK_RMENU || code == VK_LWIN || code == VK_RWIN ||
                   code == VK_INSERT || code == VK_DELETE || code == VK_HOME || code == VK_END ||
                   code == VK_PRIOR || code == VK_NEXT || code == VK_LEFT || code == VK_RIGHT ||
                   code == VK_UP || code == VK_DOWN
               ? KEYEVENTF_EXTENDEDKEY
               : 0;
}
static void release() {
    for (auto i = held.rbegin(); i != held.rend(); ++i) {
        INPUT e{};
        e.type = INPUT_KEYBOARD;
        e.ki.wVk = *i;
        e.ki.dwFlags = KEYEVENTF_KEYUP | extended(*i);
        SendInput(1, &e, sizeof(e));
    }
    held.clear();
}
static void send(INPUT *events, UINT count) {
    inputStarted = true;
    require(SendInput(count, events, sizeof(INPUT)) == count,
            L"Input was partially rejected (privilege/UIPI or desktop changed); do not replay");
}
static WORD keycode(hstring name) {
    static const std::pair<const wchar_t *, WORD> names[] = {{L"Enter", VK_RETURN},
                                                             {L"Return", VK_RETURN},
                                                             {L"Tab", VK_TAB},
                                                             {L"Escape", VK_ESCAPE},
                                                             {L"Esc", VK_ESCAPE},
                                                             {L"Space", VK_SPACE},
                                                             {L"space", VK_SPACE},
                                                             {L"Backspace", VK_BACK},
                                                             {L"BackSpace", VK_BACK},
                                                             {L"Delete", VK_DELETE},
                                                             {L"Home", VK_HOME},
                                                             {L"End", VK_END},
                                                             {L"PageUp", VK_PRIOR},
                                                             {L"PageDown", VK_NEXT},
                                                             {L"Left", VK_LEFT},
                                                             {L"ArrowLeft", VK_LEFT},
                                                             {L"Right", VK_RIGHT},
                                                             {L"ArrowRight", VK_RIGHT},
                                                             {L"Up", VK_UP},
                                                             {L"ArrowUp", VK_UP},
                                                             {L"Down", VK_DOWN},
                                                             {L"ArrowDown", VK_DOWN},
                                                             {L"Control", VK_LCONTROL},
                                                             {L"Ctrl", VK_LCONTROL},
                                                             {L"Control_L", VK_LCONTROL},
                                                             {L"Control_R", VK_RCONTROL},
                                                             {L"Shift", VK_LSHIFT},
                                                             {L"Shift_L", VK_LSHIFT},
                                                             {L"Shift_R", VK_RSHIFT},
                                                             {L"Alt", VK_LMENU},
                                                             {L"Alt_L", VK_LMENU},
                                                             {L"Alt_R", VK_RMENU},
                                                             {L"Super", VK_LWIN},
                                                             {L"Command", VK_LWIN},
                                                             {L"Super_L", VK_LWIN},
                                                             {L"Super_R", VK_RWIN},
                                                             {L"Meta", VK_LWIN},
                                                             {L"Meta_L", VK_LWIN},
                                                             {L"Meta_R", VK_RWIN}};
    for (auto entry : names)
        if (name == entry.first)
            return entry.second;
    if (name.size() == 1) {
        wchar_t c = towupper(name[0]);
        if ((c >= L'A' && c <= L'Z') || (c >= L'0' && c <= L'9'))
            return static_cast<WORD>(c);
    }
    if (name.size() >= 2 && name[0] == L'F') {
        int n = _wtoi(name.c_str() + 1);
        if (n >= 1 && n <= 24)
            return static_cast<WORD>(VK_F1 + n - 1);
    }
    throw hresult_error(E_INVALIDARG, L"key is not available: unsupported Windows key name");
}
static void input(JsonObject a, JsonObject t) {
    guardInput();
    auto kind = a.GetNamedString(L"kind");
    if (kind == L"type") {
        guardKeyboard();
        auto text = a.GetNamedString(L"text");
        for (uint32_t i = 0; i < text.size();) {
            guardInput();
            guardKeyboard();
            size_t n = 1;
            wchar_t c = text[i];
            if (c == L'\n' || c == L'\r' || c == L'\t') {
                WORD code = c == L'\t' ? VK_TAB : VK_RETURN;
                held.push_back(code);
                INPUT event{};
                event.type = INPUT_KEYBOARD;
                event.ki.wVk = code;
                send(&event, 1);
                release();
                ++i;
                continue;
            }
            if (c >= 0xd800 && c <= 0xdbff && i + 1 < text.size())
                n = 2;
            INPUT events[4]{};
            for (size_t j = 0; j < n; ++j) {
                events[2 * j].type = INPUT_KEYBOARD;
                events[2 * j].ki.wScan = text[i + j];
                events[2 * j].ki.dwFlags = KEYEVENTF_UNICODE;
                events[2 * j + 1] = events[2 * j];
                events[2 * j + 1].ki.dwFlags |= KEYEVENTF_KEYUP;
            }
            try {
                send(events, static_cast<UINT>(2 * n));
            } catch (...) {
                for (size_t j = 0; j < n; ++j)
                    SendInput(1, &events[2 * j + 1], sizeof(INPUT));
                throw;
            }
            i += static_cast<uint32_t>(n);
        }
    } else if (kind == L"key") {
        std::vector<WORD> codes;
        for (auto name : a.GetNamedArray(L"keys"))
            codes.push_back(keycode(name.GetString()));
        for (WORD code : codes) {
            guardInput();
            guardKeyboard();
            held.push_back(code);
            INPUT e{};
            e.type = INPUT_KEYBOARD;
            e.ki.wVk = code;
            e.ki.dwFlags = extended(code);
            send(&e, 1);
        }
        release();
    } else {
        auto r = t.GetNamedObject(L"desktop");
        double width = t.GetNamedNumber(L"width"), height = t.GetNamedNumber(L"height"),
               x = a.GetNamedNumber(L"x"), y = a.GetNamedNumber(L"y");
        require(x >= 0 && y >= 0 && x < width && y < height, L"Invalid screenshot coordinate");
        double px = r.GetNamedNumber(L"x") + x * r.GetNamedNumber(L"width") / width,
               py = r.GetNamedNumber(L"y") + y * r.GetNamedNumber(L"height") / height;
        require(px >= targetRect.left && px < targetRect.right && py >= targetRect.top && py < targetRect.bottom,
                L"Input outside shared source");
        int left = GetSystemMetrics(SM_XVIRTUALSCREEN), top = GetSystemMetrics(SM_YVIRTUALSCREEN),
            w = GetSystemMetrics(SM_CXVIRTUALSCREEN), h = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        require(w > 1 && h > 1 && px >= left && py >= top && px < left + w && py < top + h,
                L"Target outside current desktop geometry");
        INPUT move{};
        move.type = INPUT_MOUSE;
        move.mi.dx = static_cast<LONG>((px - left) * 65535 / (w - 1));
        move.mi.dy = static_cast<LONG>((py - top) * 65535 / (h - 1));
        move.mi.dwFlags = MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK;
        send(&move, 1);
        if (kind == L"click") {
            auto button = a.GetNamedString(L"button");
            int count = button == L"double" ? 2 : 1;
            bool right = button == L"right";
            for (int i = 0; i < count; ++i) {
                guardInput();
                INPUT e[2]{};
                e[0].type = e[1].type = INPUT_MOUSE;
                e[0].mi.dwFlags = right ? MOUSEEVENTF_RIGHTDOWN : MOUSEEVENTF_LEFTDOWN;
                e[1].mi.dwFlags = right ? MOUSEEVENTF_RIGHTUP : MOUSEEVENTF_LEFTUP;
                try {
                    send(e, 2);
                } catch (...) {
                    SendInput(1, &e[1], sizeof(INPUT));
                    throw;
                }
            }
        } else if (kind == L"scroll") {
            guardInput();
            INPUT e{};
            e.type = INPUT_MOUSE;
            e.mi.dwFlags = MOUSEEVENTF_WHEEL;
            e.mi.mouseData =
                static_cast<DWORD>(-static_cast<int>(a.GetNamedNumber(L"delta")) * WHEEL_DELTA);
            send(&e, 1);
        } else
            require(kind == L"move", L"Unsupported input action");
    }
}
static std::wstring base64(const std::vector<uint8_t> &bytes) {
    const wchar_t *alphabet = L"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    std::wstring out;
    out.reserve((bytes.size() + 2) / 3 * 4);
    for (size_t i = 0; i < bytes.size(); i += 3) {
        uint32_t v = static_cast<uint32_t>(bytes[i]) << 16;
        if (i + 1 < bytes.size())
            v |= static_cast<uint32_t>(bytes[i + 1]) << 8;
        if (i + 2 < bytes.size())
            v |= bytes[i + 2];
        out += alphabet[(v >> 18) & 63];
        out += alphabet[(v >> 12) & 63];
        out += i + 1 < bytes.size() ? alphabet[(v >> 6) & 63] : L'=';
        out += i + 2 < bytes.size() ? alphabet[v & 63] : L'=';
    }
    return out;
}
static std::vector<uint8_t> png(ID3D11Device *device, ID3D11DeviceContext *context,
                                ID3D11Texture2D *texture, int width, int height, RECT region, int outWidth, int outHeight) {
    D3D11_TEXTURE2D_DESC desc{};
    texture->GetDesc(&desc);
    require(width > 0 && height > 0 && uint64_t(width) * height <= 67108864 && uint64_t(desc.Width) * desc.Height <= 67108864,
            L"Capture dimensions exceed limit");
    require(static_cast<UINT>(width) <= desc.Width && static_cast<UINT>(height) <= desc.Height,
            L"Capture texture geometry changed");
    desc.Usage = D3D11_USAGE_STAGING;
    desc.BindFlags = 0;
    desc.MiscFlags = 0;
    desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
    com_ptr<ID3D11Texture2D> staging;
    check_hresult(device->CreateTexture2D(&desc, nullptr, staging.put()));
    context->CopyResource(staging.get(), texture);
    D3D11_MAPPED_SUBRESOURCE mapped{};
    check_hresult(context->Map(staging.get(), 0, D3D11_MAP_READ, 0, &mapped));
    Finally unmap{[&] { context->Unmap(staging.get(), 0); }};
    // Sample the mapped GPU frame directly into a bounded RGB buffer. Do not
    // allocate or encode another full-resolution CPU copy of a 4K/8K desktop.
    std::vector<BYTE> pixels(static_cast<size_t>(outWidth) * outHeight * 3);
    for (int y = 0; y < outHeight; ++y) {
        check();
        double sy = std::clamp(region.top + (y + 0.5) * (region.bottom - region.top) / outHeight - 0.5,
                               double(region.top), double(region.bottom - 1));
        int y0 = int(sy), y1 = std::min(y0 + 1, int(region.bottom - 1));
        auto row0 = static_cast<BYTE *>(mapped.pData) + size_t(y0) * mapped.RowPitch;
        auto row1 = static_cast<BYTE *>(mapped.pData) + size_t(y1) * mapped.RowPitch;
        for (int x = 0; x < outWidth; ++x) {
            double sx = std::clamp(region.left + (x + 0.5) * (region.right - region.left) / outWidth - 0.5,
                                   double(region.left), double(region.right - 1));
            int x0 = int(sx), x1 = std::min(x0 + 1, int(region.right - 1));
            for (int c = 0; c < 3; ++c) {
                double top = row0[x0 * 4 + c] * (1 - (sx - x0)) + row0[x1 * 4 + c] * (sx - x0);
                double bottom = row1[x0 * 4 + c] * (1 - (sx - x0)) + row1[x1 * 4 + c] * (sx - x0);
                pixels[(size_t(y) * outWidth + x) * 3 + c] = BYTE(top * (1 - (sy - y0)) + bottom * (sy - y0));
            }
        }
    }
    width = outWidth;
    height = outHeight;
    com_ptr<IWICImagingFactory> factory;
    check_hresult(CoCreateInstance(CLSID_WICImagingFactory, nullptr, CLSCTX_INPROC_SERVER,
                                   __uuidof(IWICImagingFactory), factory.put_void()));
    com_ptr<IStream> stream;
    check_hresult(CreateStreamOnHGlobal(nullptr, TRUE, stream.put()));
    com_ptr<IWICBitmapEncoder> encoder;
    check_hresult(factory->CreateEncoder(GUID_ContainerFormatPng, nullptr, encoder.put()));
    check_hresult(encoder->Initialize(stream.get(), WICBitmapEncoderNoCache));
    com_ptr<IWICBitmapFrameEncode> frame;
    check_hresult(encoder->CreateNewFrame(frame.put(), nullptr));
    check_hresult(frame->Initialize(nullptr));
    check_hresult(frame->SetSize(width, height));
    WICPixelFormatGUID format = GUID_WICPixelFormat24bppBGR;
    check_hresult(frame->SetPixelFormat(&format));
    require(IsEqualGUID(format, GUID_WICPixelFormat24bppBGR), L"PNG encoder changed pixel format");
    check_hresult(
        frame->WritePixels(height, width * 3, static_cast<UINT>(pixels.size()), pixels.data()));
    check_hresult(frame->Commit());
    check_hresult(encoder->Commit());
    STATSTG stat{};
    check_hresult(stream->Stat(&stat, STATFLAG_NONAME));
    require(stat.cbSize.QuadPart <= 20 * 1024 * 1024, L"PNG exceeds observation size limit");
    LARGE_INTEGER start{};
    check_hresult(stream->Seek(start, STREAM_SEEK_SET, nullptr));
    std::vector<uint8_t> result(static_cast<size_t>(stat.cbSize.QuadPart));
    ULONG read = 0;
    check_hresult(stream->Read(result.data(), static_cast<ULONG>(result.size()), &read));
    require(read == result.size(), L"Incomplete encoded image");
    return result;
}
static std::wstring name(IUIAutomationElement *e) {
    BSTR value = nullptr;
    if (FAILED(e->get_CurrentName(&value)))
        return L"";
    std::wstring result(value ? value : L"");
    SysFreeString(value);
    if (result.size() > 256)
        result.resize(256);
    if (!result.empty() && result.back() >= 0xd800 && result.back() <= 0xdbff)
        result.pop_back();
    return result;
}
static JsonArray accessibility(HWND window, RECT desktop, int width, int height) {
    com_ptr<IUIAutomation> automation;
    check_hresult(CoCreateInstance(CLSID_CUIAutomation8, nullptr, CLSCTX_INPROC_SERVER,
                                   __uuidof(IUIAutomation), automation.put_void()));
    auto settings = automation.as<IUIAutomation2>();
    check_hresult(settings->put_ConnectionTimeout(100));
    check_hresult(settings->put_TransactionTimeout(100));
    com_ptr<IUIAutomationElement> root;
    check_hresult(automation->ElementFromHandle(window, root.put()));
    com_ptr<IUIAutomationTreeWalker> walker;
    check_hresult(automation->get_ControlViewWalker(walker.put()));
    std::deque<std::pair<com_ptr<IUIAutomationElement>, unsigned>> queue;
    queue.emplace_back(root, 0);
    unsigned visited = 0;
    JsonArray elements;
    auto deadline = std::chrono::steady_clock::now() + std::chrono::milliseconds(750);
    while (!queue.empty() && visited++ < 256) {
        check();
        require(std::chrono::steady_clock::now() < deadline,
                L"UI Automation traversal deadline exceeded");
        auto [e, depth] = queue.front();
        queue.pop_front();
        BOOL password = FALSE, offscreen = TRUE;
        check_hresult(e->get_CurrentIsPassword(&password));
        if (password)
            continue;
        check_hresult(e->get_CurrentIsOffscreen(&offscreen));
        if (offscreen)
            continue;
        RECT b{};
        check_hresult(e->get_CurrentBoundingRectangle(&b));
        if (b.right > b.left && b.bottom > b.top && b.left >= desktop.left &&
            b.top >= desktop.top && b.right <= desktop.right && b.bottom <= desktop.bottom) {
            CONTROLTYPEID type = 0;
            e->get_CurrentControlType(&type);
            BOOL enabled = FALSE, focus = FALSE;
            e->get_CurrentIsEnabled(&enabled);
            e->get_CurrentHasKeyboardFocus(&focus);
            bool selected = false;
            com_ptr<IUIAutomationSelectionItemPattern> selection;
            if (SUCCEEDED(e->GetCurrentPatternAs(UIA_SelectionItemPatternId,
                                                 __uuidof(IUIAutomationSelectionItemPattern),
                                                 selection.put_void())) &&
                selection) {
                BOOL value = FALSE;
                if (SUCCEEDED(selection->get_CurrentIsSelected(&value)))
                    selected = value != FALSE;
            }
            std::wstring role = type == UIA_EditControlTypeId       ? L"entry"
                                : type == UIA_ComboBoxControlTypeId ? L"combo box"
                                : type == UIA_ButtonControlTypeId
                                    ? L"button"
                                    : L"control " + std::to_wstring(type);
            double sx = double(width) / (desktop.right - desktop.left),
                   sy = double(height) / (desktop.bottom - desktop.top);
            JsonObject item;
            item.SetNamedValue(L"id", str(L""));
            item.SetNamedValue(L"source", str(L"accessibility"));
            item.SetNamedValue(L"label", str(name(e.get())));
            item.SetNamedValue(L"role", str(role));
            item.SetNamedValue(L"bounds",
                               rectangle((b.left - desktop.left) * sx, (b.top - desktop.top) * sy,
                                         (b.right - b.left) * sx, (b.bottom - b.top) * sy));
            item.SetNamedValue(L"enabled", boolean(enabled));
            item.SetNamedValue(L"selected", boolean(selected));
            item.SetNamedValue(L"focused", boolean(focus));
            item.SetNamedValue(L"confidence", JsonValue::CreateNullValue());
            elements.Append(item);
        }
        if (depth >= 12)
            continue;
        com_ptr<IUIAutomationElement> child;
        check_hresult(walker->GetFirstChildElement(e.get(), child.put()));
        while (child && visited + queue.size() < 256) {
            check();
            require(std::chrono::steady_clock::now() < deadline,
                    L"UI Automation traversal deadline exceeded");
            queue.emplace_back(child, depth + 1);
            com_ptr<IUIAutomationElement> next;
            check_hresult(walker->GetNextSiblingElement(child.get(), next.put()));
            child = std::move(next);
        }
    }
    return elements;
}
static JsonObject capture(HWND window, bool enrich, HMONITOR monitor = nullptr, JsonObject region = nullptr) {
    check();
    require(GraphicsCaptureSession::IsSupported(), L"Windows Graphics Capture unavailable");
    RECT desktop = monitor ? monitorInfo(monitor).rcMonitor : geometry(window);
    auto interop = get_activation_factory<GraphicsCaptureItem, IGraphicsCaptureItemInterop>();
    GraphicsCaptureItem item{nullptr};
    if (monitor)
        check_hresult(interop->CreateForMonitor(monitor, guid_of<GraphicsCaptureItem>(), put_abi(item)));
    else
        check_hresult(interop->CreateForWindow(window, guid_of<GraphicsCaptureItem>(), put_abi(item)));
    com_ptr<ID3D11Device> device;
    com_ptr<ID3D11DeviceContext> context;
    D3D_FEATURE_LEVEL level;
    HRESULT hr = D3D11CreateDevice(nullptr, D3D_DRIVER_TYPE_HARDWARE, nullptr,
                                   D3D11_CREATE_DEVICE_BGRA_SUPPORT, nullptr, 0, D3D11_SDK_VERSION,
                                   device.put(), &level, context.put());
    if (FAILED(hr))
        check_hresult(D3D11CreateDevice(nullptr, D3D_DRIVER_TYPE_WARP, nullptr,
                                        D3D11_CREATE_DEVICE_BGRA_SUPPORT, nullptr, 0,
                                        D3D11_SDK_VERSION, device.put(), &level, context.put()));
    auto dxgi = device.as<IDXGIDevice>();
    com_ptr<::IInspectable> inspectable;
    check_hresult(CreateDirect3D11DeviceFromDXGIDevice(dxgi.get(), inspectable.put()));
    auto d3d = inspectable.as<IDirect3DDevice>();
    auto sourceSize = item.Size();
    require(sourceSize.Width > 0 && sourceSize.Height > 0 && uint64_t(sourceSize.Width) * sourceSize.Height <= 67108864,
            L"Display exceeds the 64-megapixel native capture budget");
    // Two buffers so a newer frame can arrive while the first is still held.
    auto pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
        d3d, DirectXPixelFormat::B8G8R8A8UIntNormalized, 2, item.Size());
    auto session = pool.CreateCaptureSession(item);
    Finally close{[&] {
        session.Close();
        pool.Close();
    }};
    if (winrt::Windows::Foundation::Metadata::ApiInformation::IsPropertyPresent(
            L"Windows.Graphics.Capture.GraphicsCaptureSession", L"IsCursorCaptureEnabled"))
        session.IsCursorCaptureEnabled(!enrich);
    session.StartCapture();
    Direct3D11CaptureFrame frame{nullptr};
    auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds(5);
    while (!(frame = pool.TryGetNextFrame())) {
        check();
        require(std::chrono::steady_clock::now() < deadline, L"Graphics Capture frame timed out");
        std::this_thread::sleep_for(std::chrono::milliseconds(10));
    }
    // A static desktop never emits a second frame; the first one stands.
    // A painting desktop replaces it until this poll ends. One session only.
    auto pollDeadline = std::chrono::steady_clock::now() + std::chrono::milliseconds(100);
    for (;;) {
        check();
        auto newer = pool.TryGetNextFrame();
        const bool arrived = static_cast<bool>(newer);
        const bool past = std::chrono::steady_clock::now() >= pollDeadline;
        switch (framePoll(true, arrived, past)) {
        case FramePoll::ReplaceAndContinue:
            frame.Close();
            frame = newer;
            break;
        case FramePoll::ReplaceAndStop:
            frame.Close();
            frame = newer;
            goto settled_frame;
        case FramePoll::Keep:
            goto settled_frame;
        case FramePoll::Wait:
            std::this_thread::sleep_for(std::chrono::milliseconds(10));
            break;
        }
    }
settled_frame:
    Finally frameClose{[&] { frame.Close(); }};
    check();
    require(same(monitor ? monitorInfo(monitor).rcMonitor : geometry(window), desktop), L"Source geometry changed during capture; observe again");
    auto size = frame.ContentSize();
    // WGC reports physical pixels. Do not invent a scale if an OS/theme returns
    // a different frame crop: its origin would be unknown and clicks would drift.
    require(size.Width == desktop.right - desktop.left &&
                size.Height == desktop.bottom - desktop.top,
            L"Graphics Capture frame does not match physical window bounds; mapping unavailable");
    auto access = frame.Surface()
                      .as<::Windows::Graphics::DirectX::Direct3D11::IDirect3DDxgiInterfaceAccess>();
    com_ptr<ID3D11Texture2D> texture;
    check_hresult(access->GetInterface(__uuidof(ID3D11Texture2D), texture.put_void()));
    RECT sample{0, 0, size.Width, size.Height};
    if (region) {
        require(monitor, L"Region capture requires a display");
        RECT requested{LONG(std::floor(region.GetNamedNumber(L"x"))), LONG(std::floor(region.GetNamedNumber(L"y"))),
            LONG(std::ceil(region.GetNamedNumber(L"x") + region.GetNamedNumber(L"width"))),
            LONG(std::ceil(region.GetNamedNumber(L"y") + region.GetNamedNumber(L"height")))};
        require(requested.left >= desktop.left && requested.top >= desktop.top && requested.right <= desktop.right && requested.bottom <= desktop.bottom
                && requested.right > requested.left && requested.bottom > requested.top, L"Region outside shared display");
        sample = {requested.left - desktop.left, requested.top - desktop.top, requested.right - desktop.left, requested.bottom - desktop.top};
        desktop = requested;
    }
    auto output = captureSize(sample.right - sample.left, sample.bottom - sample.top, !enrich);
    auto bytes = png(device.get(), context.get(), texture.get(), size.Width, size.Height, sample, int(output.first), int(output.second));
    size.Width = int(output.first);
    size.Height = int(output.second);
    JsonArray elements;
    hstring status = L"UI Automation selected-window labels/roles/bounds; 256 nodes, depth 12, 750 "
                     L"ms; password values omitted";
    try {
        if (monitor) status = L"unavailable: display observations use visible pixels and OCR; window-only UI Automation targets may be occluded";
        else if (enrich) elements = accessibility(window, desktop, size.Width, size.Height);
    } catch (const hresult_error &e) {
        status = L"unavailable: " + e.message();
    }
    JsonObject transform;
    transform.SetNamedValue(L"desktop", rectangle(desktop));
    transform.SetNamedValue(L"width", num(size.Width));
    transform.SetNamedValue(L"height", num(size.Height));
    JsonObject result;
    result.SetNamedValue(L"png", str(base64(bytes)));
    result.SetNamedValue(L"transform", transform);
    result.SetNamedValue(L"elements", elements);
    result.SetNamedValue(L"accessibility_status", str(status));
    result.SetNamedValue(L"ocr_status", str(L"pending local Tesseract"));
    return result;
}
static IJsonValue dispatch(JsonObject r) {
    auto command = r.GetNamedString(L"command");
    if (command == L"release") {
        release();
        return JsonValue::CreateNullValue();
    }
    if (command == L"reset") {
        release();
        cancelled = false;
        target = nullptr;
        targetMonitor = nullptr;
        return JsonValue::CreateNullValue();
    }
    if (command == L"capabilities") {
        bool supported = false;
        try {
            auto factory =
                get_activation_factory<GraphicsCaptureItem, IGraphicsCaptureItemInterop>();
            supported = bool(factory) && GraphicsCaptureSession::IsSupported() && localSession();
        } catch (...) {
        }
        JsonObject result;
        for (auto key :
             {L"capture", L"windows", L"focus", L"pointer", L"keyboard"})
            result.SetNamedValue(key, boolean(supported));
        result.SetNamedValue(L"accessibility", boolean(supported));
        result.SetNamedValue(L"ocr", boolean(false));
        result.SetNamedValue(L"floating", boolean(true));
        JsonArray limits;
        if (!supported)
            limits.Append(str(L"Computer use requires a local Windows 10 1903+ console with "
                              L"Graphics Capture support"));
        limits.Append(str(L"Secure desktop, protected content and elevated applications can reject "
                          L"capture/input; no privilege escalation is attempted"));
        limits.Append(str(L"Screen shares permit native input. Application windows are view-only assist mode; UI Automation metadata applies only to application observations."));
        result.SetNamedValue(L"limitations", limits);
        return result;
    }
    check();
    require(localSession(), L"Computer use requires the local console session");
    desktopAvailable();
    if (command == L"windows") {
        JsonArray windows;
        for (HMONITOR monitor : monitors()) {
            check();
            windows.Append(describeMonitor(monitor));
        }
        EnumWindows(
            [](HWND w, LPARAM value) -> BOOL {
                auto out = reinterpret_cast<JsonArray *>(value);
                if (cancelled.load() || out->Size() >= 256)
                    return FALSE;
                DWORD pid = 0;
                GetWindowThreadProcessId(w, &pid);
                // Owned top-level dialogs are selectable targets too. EnumWindows
                // already excludes ordinary child controls.
                if (!pid || pid == GetCurrentProcessId() || !IsWindowVisible(w) || IsIconic(w))
                    return TRUE;
                DWORD cloaked = 0;
                DwmGetWindowAttribute(w, DWMWA_CLOAKED, &cloaked, sizeof(cloaked));
                if (cloaked)
                    return TRUE;
                try {
                    out->Append(describe(w));
                } catch (...) {
                }
                return TRUE;
            },
            reinterpret_cast<LPARAM>(&windows));
        check();
        return windows;
    }
    if (command == L"inspect" || command == L"select") {
        auto id = r.GetNamedString(L"window");
        if (isDisplay(id)) {
            targetMonitor = lookupMonitor(id);
            targetRect = monitorInfo(targetMonitor).rcMonitor;
            targetId = monitorId(targetMonitor);
            return describeMonitor(targetMonitor);
        }
        targetMonitor = nullptr;
        HWND w = lookup(id);
        target = w;
        targetRect = geometry(w);
        targetTitle = title(w);
        targetId = identity(w);
        return describe(w);
    }
    if (command == L"capture" || command == L"preview") {
        auto id = r.GetNamedString(L"window");
        return isDisplay(id) ? capture(nullptr, command == L"capture", lookupMonitor(id), r.HasKey(L"region") ? r.GetNamedObject(L"region") : nullptr)
                             : capture(lookup(id), command == L"capture");
    }
    if (command == L"validate_input") {
        guardInput();
        auto kind = r.GetNamedObject(L"action").GetNamedString(L"kind");
        if (kind == L"type" || kind == L"key") guardKeyboard();
        if (kind == L"key")
            for (auto name : r.GetNamedObject(L"action").GetNamedArray(L"keys")) keycode(name.GetString());
        return JsonValue::CreateNullValue();
    }
    if (command == L"input") {
        try {
            input(r.GetNamedObject(L"action"), r.GetNamedObject(L"transform"));
        } catch (...) {
            release();
            throw;
        }
        return JsonValue::CreateNullValue();
    }
    throw hresult_error(E_INVALIDARG, L"Unknown native desktop command");
}
extern "C" char *koma_computer_call(const char *json) {
    inputStarted = false;
    try {
        HRESULT apartment = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
        Finally uninit{[&] {
            if (SUCCEEDED(apartment))
                CoUninitialize();
        }};
        check_hresult(apartment);
        using SetDpi = DPI_AWARENESS_CONTEXT(WINAPI *)(DPI_AWARENESS_CONTEXT);
        auto setDpi = reinterpret_cast<SetDpi>(
            GetProcAddress(GetModuleHandleW(L"user32.dll"), "SetThreadDpiAwarenessContext"));
        auto previous = setDpi ? setDpi(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) : nullptr;
        Finally dpi{[&] {
            if (setDpi && previous)
                setDpi(previous);
        }};
        JsonObject response;
        try {
            response.SetNamedValue(L"result", dispatch(JsonObject::Parse(to_hstring(json))));
        } catch (const InputBusy &) {
            response.SetNamedValue(L"error", str(L"Keyboard or mouse is busy"));
            response.SetNamedValue(L"input_busy", boolean(true));
            response.SetNamedValue(L"input_started", boolean(inputStarted));
        } catch (const hresult_error &e) {
            response.SetNamedValue(L"error", str(e.message()));
        } catch (const std::exception &e) {
            response.SetNamedValue(L"error", JsonValue::CreateStringValue(to_hstring(e.what())));
        } catch (...) {
            response.SetNamedValue(L"error", str(L"Native desktop SDK exception"));
        }
        auto text = to_string(response.Stringify());
        char *out = static_cast<char *>(std::malloc(text.size() + 1));
        if (out)
            std::memcpy(out, text.c_str(), text.size() + 1);
        return out;
    } catch (...) {
        // No C++/WinRT exception may cross the Rust C ABI, including activation
        // failures on Windows versions without the required runtime classes.
        return _strdup("{\"error\":\"Windows desktop runtime unavailable\"}");
    }
}
extern "C" void koma_computer_free(char *reply) { std::free(reply); }
extern "C" void koma_computer_cancel() { cancelled = true; }
