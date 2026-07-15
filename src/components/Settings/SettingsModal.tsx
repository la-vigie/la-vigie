import { useEffect, useState } from "react";
import { PromptManager } from "../Prompts/PromptManager";
import { AgentsTab } from "./AgentsTab";
import { NotificationsTab } from "./NotificationsTab";
import { RemoteTab } from "./RemoteTab";
import { GeneralTab } from "./GeneralTab";
import "./SettingsModal.css";

interface SettingsModalProps {
  onClose: () => void;
}

type SettingsTab = "agents" | "notifications" | "remote" | "general" | "prompts";

const TABS: { id: SettingsTab; label: string }[] = [
  { id: "agents", label: "Agents" },
  { id: "notifications", label: "Notifications" },
  { id: "remote", label: "Remote" },
  { id: "general", label: "General" },
  { id: "prompts", label: "Prompts" },
];

export function SettingsModal({ onClose }: SettingsModalProps) {
  const [tab, setTab] = useState<SettingsTab>("agents");

  // Escape to close
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div className="settings__backdrop" role="presentation" onClick={onClose}>
      <div
        className="settings"
        role="dialog"
        aria-label="Settings"
        onClick={(e) => e.stopPropagation()}
      >
        <header className="settings__header">
          <h2 className="settings__title">Settings</h2>
          <button
            type="button"
            className="icon-btn settings__close"
            aria-label="Close"
            onClick={onClose}
          >
            ✕
          </button>
        </header>

        <div className="settings__tabs" role="tablist">
          {TABS.map(({ id, label }) => (
            <button
              key={id}
              type="button"
              role="tab"
              aria-selected={tab === id}
              className={`settings__tab${tab === id ? " settings__tab--active" : ""}`}
              onClick={() => setTab(id)}
            >
              {label}
            </button>
          ))}
        </div>

        <div className="settings__body">
          {tab === "agents" && <AgentsTab />}
          {tab === "notifications" && <NotificationsTab />}
          {tab === "remote" && <RemoteTab />}
          {tab === "general" && <GeneralTab />}
          {tab === "prompts" && <PromptManager />}
        </div>
      </div>
    </div>
  );
}
