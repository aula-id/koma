// Portable unit check; no OS input/capture. Compile with C++17 and run.
#include "input_idle.h"
#include <cassert>
#include <stdexcept>

int main() {
    int polls = 0;
    waitForInputIdle([] {}, [&] { return ++polls < 3; });
    assert(polls == 3); // Transient queued key-up state clears without failure.
    bool busy = false;
    const auto start = std::chrono::steady_clock::now();
    try { waitForInputIdle([] {}, [] { return true; }); }
    catch (const InputBusy &) { busy = true; }
    assert(busy);
    assert(std::chrono::steady_clock::now() - start >= std::chrono::milliseconds(250));
    polls = 0;
    bool cancelled = false;
    try {
        waitForInputIdle([&] { if (++polls == 2) throw std::runtime_error("cancelled"); }, [] { return true; });
    } catch (const std::runtime_error &) { cancelled = true; }
    assert(cancelled && polls == 2);
}
