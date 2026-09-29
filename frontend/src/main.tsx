import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import App from './App'
import { fly } from './tauri/shim'
import './styles.css'

window.fly = fly

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>
)
