/**
 * The sounds of a call, made with Web Audio (no files to load): a two-note
 * chime that repeats while a call comes in, a soft single tone while one
 * goes out. Browsers only allow sound after the person has touched the page;
 * until then ringing is silent and the sheet alone announces the call.
 */

type Pattern = { notes: number[]; noteMs: number; everyMs: number; volume: number };

const PATTERNS: Record<"incoming" | "outgoing", Pattern> = {
  incoming: { notes: [880, 1174.66, 880, 1174.66], noteMs: 150, everyMs: 2200, volume: 0.16 },
  outgoing: { notes: [440], noteMs: 900, everyMs: 3000, volume: 0.07 },
};

/** Starts ringing; call the returned function to stop. */
export function ring(kind: "incoming" | "outgoing"): () => void {
  const AudioContextClass = typeof window === "undefined" ? undefined : window.AudioContext;
  if (!AudioContextClass) return () => {};
  let context: AudioContext;
  try {
    context = new AudioContextClass();
  } catch {
    return () => {};
  }
  const pattern = PATTERNS[kind];

  const play = () => {
    if (context.state !== "running") return;
    pattern.notes.forEach((frequency, index) => {
      const start = context.currentTime + (index * pattern.noteMs) / 1000;
      const end = start + pattern.noteMs / 1000;
      const oscillator = context.createOscillator();
      const gain = context.createGain();
      oscillator.type = "sine";
      oscillator.frequency.value = frequency;
      // A short fade at both ends keeps the notes from clicking.
      gain.gain.setValueAtTime(0, start);
      gain.gain.linearRampToValueAtTime(pattern.volume, start + 0.02);
      gain.gain.setValueAtTime(pattern.volume, end - 0.04);
      gain.gain.linearRampToValueAtTime(0, end);
      oscillator.connect(gain).connect(context.destination);
      oscillator.start(start);
      oscillator.stop(end);
    });
  };

  void context
    .resume()
    .then(play)
    .catch(() => {});
  const timer = setInterval(play, pattern.everyMs);
  return () => {
    clearInterval(timer);
    void context.close().catch(() => {});
  };
}
