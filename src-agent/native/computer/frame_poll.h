#pragma once

// One Graphics Capture session. After the first frame is held, keep any newer
// frame that arrives until the poll deadline, then keep that newest frame.
enum class FramePoll { Wait, Keep, ReplaceAndContinue, ReplaceAndStop };

inline FramePoll framePoll(bool held, bool arrived, bool past) {
    if (arrived && past)
        return FramePoll::ReplaceAndStop;
    if (arrived)
        return FramePoll::ReplaceAndContinue;
    if (held && past)
        return FramePoll::Keep;
    return FramePoll::Wait;
}
