import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { AlertTriangle, ExternalLink } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import { Button } from '@/components/ui/button';
import { usePlatform } from '@/platform/PlatformContext';

/**
 * Non-blocking banner that pops up when macOS Accessibility permission is
 * missing. Without this permission the global chord can still record, but
 * the synthetic-⌘V paste silently drops — so we need an inline prompt
 * rather than relying on the system-level permission dialog (which only
 * fires once, the first time the app tries to post a keystroke).
 *
 * Triggered on three signals:
 * - app mount in Tauri
 * - `system:accessibility-missing` event from the dictate window's paste
 *   failure handler
 * - window focus (cheap way to re-check after the user flips the toggle
 *   in System Settings and alt-tabs back)
 *
 * Hides itself automatically the next time the check returns `true`.
 */
export function AccessibilityGate() {
  const platform = usePlatform();
  const [needsPermission, setNeedsPermission] = useState(false);
  const [checking, setChecking] = useState(false);

  const recheck = useCallback(async () => {
    if (!platform.metadata.isTauri) return;
    setChecking(true);
    try {
      const trusted = await invoke<boolean>('check_accessibility_permission');
      setNeedsPermission(!trusted);
    } catch (err) {
      console.warn('[accessibility] check failed:', err);
    } finally {
      setChecking(false);
    }
  }, [platform.metadata.isTauri]);

  // Initial check + refresh whenever the window regains focus. Users who
  // just flipped the toggle in System Settings expect the banner to go
  // away as soon as they come back to Voicebox.
  useEffect(() => {
    if (!platform.metadata.isTauri) return;
    recheck();
    const onFocus = () => {
      recheck();
    };
    window.addEventListener('focus', onFocus);
    return () => window.removeEventListener('focus', onFocus);
  }, [platform.metadata.isTauri, recheck]);

  // React to the dictate window's paste failures — the paste pipeline
  // emits this event when `AXIsProcessTrusted()` returns false so the
  // user doesn't have to guess why their dictation didn't land.
  useEffect(() => {
    if (!platform.metadata.isTauri) return;
    let unlisten: UnlistenFn | null = null;
    listen('system:accessibility-missing', () => {
      setNeedsPermission(true);
    })
      .then((fn) => {
        unlisten = fn;
      })
      .catch(() => {});
    return () => {
      if (unlisten) unlisten();
    };
  }, [platform.metadata.isTauri]);

  const openSettings = useCallback(async () => {
    try {
      await invoke('open_accessibility_settings');
    } catch (err) {
      console.warn('[accessibility] open settings failed:', err);
    }
  }, []);

  if (!needsPermission) return null;

  return (
    <div className="shrink-0 bg-amber-500/10 border-b border-amber-500/30">
      <div className="flex items-start gap-3 px-4 py-3 max-w-5xl mx-auto">
        <AlertTriangle className="h-4 w-4 shrink-0 mt-0.5 text-amber-500" />
        <div className="flex-1 min-w-0 space-y-1">
          <p className="text-sm font-medium text-foreground">
            Grant Accessibility permission to enable auto-paste
          </p>
          <p className="text-sm text-muted-foreground leading-relaxed">
            Voicebox needs System Settings → Privacy &amp; Security → Accessibility
            to paste transcriptions into other apps. Your dictation still lands
            in the Captures tab without it.
          </p>
        </div>
        <div className="flex items-center gap-2 shrink-0">
          <Button variant="outline" size="sm" onClick={recheck} disabled={checking}>
            I've enabled it
          </Button>
          <Button size="sm" onClick={openSettings} className="gap-1.5">
            <ExternalLink className="h-3.5 w-3.5" />
            Open Settings
          </Button>
        </div>
      </div>
    </div>
  );
}
