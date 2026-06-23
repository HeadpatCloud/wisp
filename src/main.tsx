import React from 'react'
import ReactDOM from 'react-dom/client'
import './index.css'
import { ErrorBoundary } from '@/components/ErrorBoundary'
import { applyAppTheme, cachedTheme } from '@/lib/theme'
import App from './App'

applyAppTheme(cachedTheme())

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </React.StrictMode>,
)
