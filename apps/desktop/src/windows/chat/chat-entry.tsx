import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import '../../shared/styles/global.css';
import { ChatWindow } from './ChatWindow';

const container = document.getElementById('root');
if (!container) {
  throw new Error('chat window root element is missing');
}

createRoot(container).render(
  <StrictMode>
    <ChatWindow />
  </StrictMode>,
);
