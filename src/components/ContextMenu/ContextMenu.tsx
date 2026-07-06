import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import "./ContextMenu.css";

export interface ContextMenuItem {
  label: string;
  onSelect: () => void;
  danger?: boolean;
}

interface ContextMenuProps {
  items: ContextMenuItem[];
  position: { x: number; y: number };
  onClose: () => void;
}

export function ContextMenu({ items, position, onClose }: ContextMenuProps) {
  const menuRef = useRef<HTMLDivElement>(null);
  const [active, setActive] = useState(0);
  const [pos, setPos] = useState(position);

  // Clamp/flip within the viewport once we can measure the menu.
  useLayoutEffect(() => {
    const el = menuRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    let { x, y } = position;
    if (x + rect.width > window.innerWidth) x = Math.max(0, window.innerWidth - rect.width);
    if (y + rect.height > window.innerHeight) y = Math.max(0, window.innerHeight - rect.height);
    setPos({ x, y });
  }, [position]);

  // Move focus into the menu so keyboard nav works immediately.
  // Capture the element that had focus before we opened, so we can restore it
  // when the menu unmounts (Escape, item select, or outside click).
  useEffect(() => {
    const opener = document.activeElement;
    menuRef.current?.focus();
    return () => {
      if (opener instanceof HTMLElement && opener.isConnected) {
        opener.focus();
      }
    };
  }, []);

  // Dismiss on outside-click, scroll, and window resize.
  useEffect(() => {
    const onPointerDown = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) onClose();
    };
    window.addEventListener("mousedown", onPointerDown);
    window.addEventListener("scroll", onClose, true);
    window.addEventListener("resize", onClose);
    return () => {
      window.removeEventListener("mousedown", onPointerDown);
      window.removeEventListener("scroll", onClose, true);
      window.removeEventListener("resize", onClose);
    };
  }, [onClose]);

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      setActive((i) => (i + 1) % items.length);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((i) => (i - 1 + items.length) % items.length);
    } else if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      const item = items[active];
      if (item) {
        item.onSelect();
        onClose();
      }
    }
  };

  return createPortal(
    <div
      ref={menuRef}
      className="context-menu"
      role="menu"
      aria-label="Context menu"
      aria-activedescendant={`ctx-item-${active}`}
      tabIndex={-1}
      style={{ left: pos.x, top: pos.y }}
      onKeyDown={onKeyDown}
    >
      {items.map((item, i) => (
        <button
          key={i}
          id={`ctx-item-${i}`}
          type="button"
          role="menuitem"
          className={
            "context-menu__item" +
            (item.danger ? " context-menu__item--danger" : "") +
            (i === active ? " context-menu__item--active" : "")
          }
          onMouseEnter={() => setActive(i)}
          onClick={() => {
            item.onSelect();
            onClose();
          }}
        >
          {item.label}
        </button>
      ))}
    </div>,
    document.body,
  );
}
