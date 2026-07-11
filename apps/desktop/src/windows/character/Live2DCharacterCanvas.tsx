import { useEffect, useRef } from 'react';

export function Live2DCharacterCanvas() {
  const ref = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const canvas = ref.current;
    if (!canvas || !import.meta.env.DEV) return;
    let active = true;
    let dispose: (() => void) | undefined;
    void import('./Live2DCharacterDev').then(module => {
      const cleanup = module.mountLive2DCharacterDev(canvas);
      if (active) dispose = cleanup; else cleanup();
    });
    return () => { active = false; dispose?.(); };
  }, []);
  return <canvas ref={ref} className="character-stage__live2d" aria-label="Live2D character" />;
}
