import { inject } from '@vercel/analytics';

if (typeof window !== 'undefined') {
  try {
    inject({
      debug: false,
    });
  } catch (err) {
    console.warn('[Vercel Analytics] Failed to initialize:', err);
  }
}
