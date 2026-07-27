#!/usr/bin/env python3
"""Find the beat grid of a raw CD-DA track.

The menu pulses its visuals on the beat, driven by `psx_io::cdda::CddaClock`,
which reports milliseconds into the playing track. To turn that into beats the
launcher needs a tempo and a phase, and both have to be accurate: a tempo off
by 1 BPM drifts several beats out of step over a four-minute track, which
reads as broken rather than as an effect.

So this measures rather than trusts the tempo the artist quoted. It builds an
onset-strength envelope, scores every (tempo, phase) pair on a fine grid by
how much onset energy lands on the beats, and prints the winner in the form
`mkdisc --menu-beat` wants.

    python3 tools/beatgrid.py audio/*.cdda

Input is raw 44.1 kHz 16-bit stereo PCM, which is what the disc carries.
"""

import struct
import sys

SAMPLE_RATE = 44100
HOP = 256  # ~5.8 ms, fine enough to place a beat inside one frame at 60 Hz


def onset_envelope(path):
    """Half-wave rectified energy difference, one value per HOP samples."""
    raw = open(path, "rb").read()
    frames = len(raw) // 4
    samples = struct.unpack("<%dh" % (frames * 2), raw[: frames * 4])
    energy = []
    for start in range(0, frames - HOP, HOP):
        total = 0
        for i in range(start, start + HOP):
            left = samples[2 * i]
            right = samples[2 * i + 1]
            total += abs(left) + abs(right)
        energy.append(total / (2 * HOP))
    return [max(0.0, energy[i] - energy[i - 1]) for i in range(1, len(energy))]


def score(envelope, rate, bpm, phase_frames):
    """Total onset strength landing on a grid at `bpm` starting `phase_frames` in."""
    spacing = rate * 60.0 / bpm
    total = 0.0
    beat = 0
    while True:
        at = phase_frames + beat * spacing
        index = int(at + 0.5)
        if index >= len(envelope):
            return total
        # A beat rarely lands exactly on a frame boundary, so take the best of
        # the frame and its neighbours.
        total += max(envelope[max(0, index - 1) : index + 2])
        beat += 1


def beat_grid(path):
    envelope = onset_envelope(path)
    rate = SAMPLE_RATE / HOP
    mean = sum(envelope) / len(envelope)
    envelope = [max(0.0, e - mean) for e in envelope]

    # Coarse pass over plausible drum and bass tempos, then refine around the
    # winner. A single fine sweep would be minutes of work for the same answer.
    best = None
    for tenths in range(1600, 1900):
        bpm = tenths / 10
        spacing = rate * 60.0 / bpm
        for phase in range(0, int(spacing) + 1, 2):
            value = score(envelope, rate, bpm, phase)
            if best is None or value > best[0]:
                best = (value, bpm, phase)

    _, coarse_bpm, coarse_phase = best
    best = None
    for milli in range(int(coarse_bpm * 1000) - 400, int(coarse_bpm * 1000) + 401, 10):
        bpm = milli / 1000
        for phase in range(max(0, coarse_phase - 4), coarse_phase + 5):
            value = score(envelope, rate, bpm, phase)
            if best is None or value > best[0]:
                best = (value, milli, phase)

    _, milli_bpm, phase_frames = best
    phase_ms = int(phase_frames * HOP * 1000 / SAMPLE_RATE)
    return milli_bpm, phase_ms


def main(paths):
    if not paths:
        print(__doc__)
        return 1
    for path in paths:
        milli_bpm, phase_ms = beat_grid(path)
        print(
            "%-28s %7.3f BPM, first beat %4d ms   --menu-beat %d:%d"
            % (path, milli_bpm / 1000, phase_ms, milli_bpm, phase_ms)
        )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
