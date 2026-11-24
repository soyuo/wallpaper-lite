import { APP_PAGES } from "./app-pages";
import type { AppPage } from "./app-pages";

export function AppDock({ page, editorDirty, onNavigate }: { page: AppPage; editorDirty: boolean; onNavigate(page: AppPage): void }) {
  return (
    <nav className="app-dock" aria-label="Main">
      <ul>
        {APP_PAGES.map((item) => (
          <li key={item.id}>
            <button className="app-dock-item" aria-current={page === item.id ? "page" : undefined} onClick={() => onNavigate(item.id)}>
              <span className="app-dock-icon" aria-hidden="true">{item.icon}</span>
              <span>{item.label}</span>
              {item.id === "editor" && editorDirty && <span className="app-nav-dirty" role="img" aria-label="Unapplied changes"> •</span>}
            </button>
          </li>
        ))}
      </ul>
    </nav>
  );
}
