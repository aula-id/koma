#pragma once
#include <chrono>
#include <thread>

struct InputBusy {};

// A short, cancellable settling window also lets our queued key-up events reach
// the OS state table before the next character or action checks it.
template <typename Check, typename Busy>
static void waitForInputIdle(Check check, Busy busy) {
    const auto deadline = std::chrono::steady_clock::now() + std::chrono::milliseconds(250);
    for (;;) {
        check();
        if (!busy()) return;
        if (std::chrono::steady_clock::now() >= deadline) throw InputBusy{};
        std::this_thread::sleep_for(std::chrono::milliseconds(10));
    }
}
