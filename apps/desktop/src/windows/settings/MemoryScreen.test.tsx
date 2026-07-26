import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { MemoryScreen } from './MemoryScreen';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue(null) }));

describe('MemoryScreen', () => {
  it('renders the six memory sections in the specified order', () => {
    render(<MemoryScreen />);
    expect(screen.getAllByRole('heading', { level: 2 }).map((heading) => heading.textContent))
      .toEqual([
        'あなたについて',
        'シークレットモード',
        'タスク',
        '保存済みのメモリー',
        'インポート/エクスポート',
        '削除',
      ]);
  });
});
