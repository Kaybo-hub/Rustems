// hooks/useThrottle.ts
import { useRef, useCallback } from 'react';

export function useThrottle(callback: (...args: any[]) => void, delay: number) {
  const lastCall = useRef(0);
  
  return useCallback((...args: any[]) => {
    const now = new Date().getTime();
    if (now - lastCall.current >= delay) {
      lastCall.current = now;
      callback(...args);
    }
  }, [callback, delay]);
}