// noVNC без собственных типов: минимальное описание того, что использует VncView
declare module "@novnc/novnc/lib/rfb" {
  export default class RFB extends EventTarget {
    constructor(target: HTMLElement, urlOrChannel: unknown, options?: Record<string, unknown>);
    scaleViewport: boolean;
    resizeSession: boolean;
    focusOnClick: boolean;
    viewOnly: boolean;
    disconnect(): void;
    focus(): void;
    sendCredentials(creds: { username?: string; password?: string }): void;
    clipboardPasteFrom(text: string): void;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    addEventListener(type: string, listener: (e: any) => void): void;
  }
}
