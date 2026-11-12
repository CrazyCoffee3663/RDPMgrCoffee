import { app } from "./state.svelte";

export type Dialog = {
  id: number;
  kind: "confirm" | "input" | "select";
  title: string;
  message: string;
  value: string;
  okText: string;
  danger: boolean;
  /** Дополнительная кнопка (возвращает "alt") */
  altText?: string;
  /** Варианты для kind = "select": [значение, подпись] */
  options?: [string, string][];
  resolve: (v: string | null) => void;
};

let seq = 0;

class Dialogs {
  list = $state<Dialog[]>([]);

  private open(d: Omit<Dialog, "id" | "resolve">): Promise<string | null> {
    app.pushOverlay();
    return new Promise((resolve) => {
      const id = ++seq;
      this.list.push({
        ...d,
        id,
        resolve: (v) => {
          this.list = this.list.filter((x) => x.id !== id);
          app.popOverlay();
          resolve(v);
        },
      });
    });
  }

  async confirm(title: string, message: string, okText = "OK", danger = false): Promise<boolean> {
    return (await this.open({ kind: "confirm", title, message, value: "", okText, danger })) !== null;
  }

  /** Выбор из двух действий: "ok" | "alt" | null (отмена) */
  choose(title: string, message: string, okText: string, altText: string, danger = false): Promise<string | null> {
    return this.open({ kind: "confirm", title, message, value: "", okText, altText, danger });
  }

  /** Выбор из списка; возвращает значение варианта или null */
  select(title: string, message: string, options: [string, string][], value = "", okText = "OK"): Promise<string | null> {
    return this.open({ kind: "select", title, message, value: value || options[0]?.[0] || "", okText, options, danger: false });
  }

  input(title: string, message: string, value = "", okText = "OK"): Promise<string | null> {
    return this.open({ kind: "input", title, message, value, okText, danger: false });
  }
}

export const dialogs = new Dialogs();

// ---------- Контекстное меню ----------

export type MenuItem =
  | { label: string; icon?: string; danger?: boolean; disabled?: boolean; action: () => void; hint?: string }
  | "sep";

class Menu {
  open = $state(false);
  x = $state(0);
  y = $state(0);
  items = $state<MenuItem[]>([]);

  show(e: MouseEvent, items: MenuItem[]) {
    e.preventDefault();
    e.stopPropagation();
    if (!this.open) app.pushOverlay();
    this.items = items;
    this.x = e.clientX;
    this.y = e.clientY;
    this.open = true;
  }

  close() {
    if (this.open) {
      this.open = false;
      app.popOverlay();
    }
  }
}

export const menu = new Menu();
