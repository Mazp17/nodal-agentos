import { ProjectsProvider } from "./domain/hooks/projects";
import { AppShell } from "./shell/AppShell";
import { ProviderStatusProvider } from "./shell/providerStatus";
import { ToastProvider } from "./ui/Toasts";

export default function App() {
  return (
    <ToastProvider>
      <ProjectsProvider>
        <ProviderStatusProvider>
          <AppShell />
        </ProviderStatusProvider>
      </ProjectsProvider>
    </ToastProvider>
  );
}
