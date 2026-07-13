import type { ThemePreferenceDto } from '@parallel-world/contracts';

export function applyThemePreference(theme: ThemePreferenceDto) {
  if (theme === 'system') {
    document.documentElement.removeAttribute('data-theme');
  } else {
    document.documentElement.dataset.theme = theme;
  }
}
