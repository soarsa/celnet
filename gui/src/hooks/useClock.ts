/**
 * A shared 1 Hz clock for the status ribbon and a high-rate now-nanos sampler
 * for last-look countdowns. Both are React-friendly and tear down cleanly.
 */

import { useEffect, useRef, useState } from "react";

const NS_PER_MS = 1_000_000n;

/** Current wall-clock in nanoseconds since the Unix epoch. */
export function nowNanos(): bigint {
  return BigInt(Math.round(performance.timeOrigin + performance.now())) * NS_PER_MS;
}

/** A clock that re-renders once per second (the ribbon clock). */
export function useSecondClock(): bigint {
  const [, setTick] = useState(0);
  useEffect(() => {
    const id = setInterval(() => setTick((t) => t + 1), 1000);
    return () => clearInterval(id);
  }, []);
  return nowNanos();
}

/**
 * A countdown clock that re-renders on an animation frame while `active`, used
 * by the last-look ring so the depletion is smooth (and stops when inactive to
 * respect the frame budget).
 */
export function useCountdownClock(active: boolean): bigint {
  const [now, setNow] = useState<bigint>(() => nowNanos());
  const raf = useRef<number>(0);
  useEffect(() => {
    if (!active) return;
    let mounted = true;
    const loop = () => {
      if (!mounted) return;
      setNow(nowNanos());
      raf.current = requestAnimationFrame(loop);
    };
    raf.current = requestAnimationFrame(loop);
    return () => {
      mounted = false;
      cancelAnimationFrame(raf.current);
    };
  }, [active]);
  return now;
}
