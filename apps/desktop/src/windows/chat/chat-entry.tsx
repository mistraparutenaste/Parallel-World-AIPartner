import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { ChatWindow } from './ChatWindow';

const container = document.getElementById('root');
if (!container) throw new Error('Missing root element');

createRoot(container, {
  onCaughtError: (error, errorInfo) => console.error('Caught React error', error, errorInfo),
  onUncaughtError: (error, errorInfo) => console.error('Uncaught React error', error, errorInfo),
}).render(<StrictMode><ChatWindow /></StrictMode>);
