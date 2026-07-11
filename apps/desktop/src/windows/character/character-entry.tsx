import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import '../../shared/styles/global.css';
import { CharacterWindow } from './CharacterWindow';

document.body.classList.add('transparent');

const container = document.getElementById('root');
if (!container) {
  throw new Error('character window root element is missing');
}

createRoot(container).render(
  <StrictMode>
    <CharacterWindow />
  </StrictMode>,
);
