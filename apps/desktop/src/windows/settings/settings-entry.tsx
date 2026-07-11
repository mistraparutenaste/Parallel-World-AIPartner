import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import '../../shared/styles/global.css';
import { SettingsWindow } from './SettingsWindow';

const container = document.getElementById('root');
if (!container) {
  throw new Error('settings window root element is missing');
}

createRoot(container).render(
  <StrictMode>
    <SettingsWindow />
  </StrictMode>,
);
