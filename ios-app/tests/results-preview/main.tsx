import React from 'react';
import { createRoot } from 'react-dom/client';
import MetricsScreen from '../../app/metrics';
import './preview.css';

const scale = new URLSearchParams(location.search).get('text') === 'large' ? 1.35 : 1;
// Browser approximation of larger text: enlarge actual rendered text metrics
// without scaling the viewport, chart geometry or touch-target dimensions.
if (scale !== 1) {
  const scaleText = () => {
    for (const element of document.querySelectorAll<HTMLElement>('#screen [dir="auto"], #screen input')) {
      if (element.dataset.previewScaled) continue;
      const style = getComputedStyle(element);
      const size = Number.parseFloat(style.fontSize), height = Number.parseFloat(style.lineHeight);
      element.style.fontSize = `${size * scale}px`;
      if (Number.isFinite(height)) element.style.lineHeight = `${height * scale}px`;
      element.dataset.previewScaled = 'true';
    }
  };
  new MutationObserver(scaleText).observe(document.getElementById('root')!, { childList: true, subtree: true });
}

createRoot(document.getElementById('root')!).render(<>
  <aside id="fixture-banner">ILLUSTRATIVE FIXTURE · UI validation only · no measured phone results{scale > 1 ? ' · 135% text' : ''}</aside>
  <div id="screen"><MetricsScreen /></div>
</>);
