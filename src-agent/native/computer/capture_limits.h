#pragma once
#include <algorithm>
#include <cmath>
#include <cstddef>
#include <utility>

// Keep in sync with computer::contract::capture_size. Bound the image before
// PNG encoding/IPC/OCR, not only before it reaches the model.
static inline std::pair<size_t, size_t> captureSize(size_t width, size_t height, bool preview) {
    if (!width || !height) return {0, 0};
    const double maxWidth = preview ? 1280 : 1920;
    const double maxHeight = preview ? 960 : 1920;
    const double pixels = preview ? 1228800 : 2073600;
    const double scale = std::min({1.0, maxWidth / width, maxHeight / height,
        std::sqrt(pixels / (double(width) * double(height)))});
    return {std::max<size_t>(1, size_t(std::floor(width * scale))),
            std::max<size_t>(1, size_t(std::floor(height * scale)))};
}
