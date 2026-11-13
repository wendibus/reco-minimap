import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import './trainer-globals.css';
import './trainer-theme.css';
import './minimap.css';
import { App } from './App';
import { AppProvider } from './core';

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <AppProvider><App /></AppProvider>
  </StrictMode>,
);
