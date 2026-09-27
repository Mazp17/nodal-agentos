import { LaunchConfigProvider } from "./features/tasks/LaunchPopover";
import { AppShell } from "./shell/AppShell";
import { ConfirmProvider } from "./ui/ConfirmDialog";
import { ToastProvider } from "./ui/Toasts";

export default function App() {
  return (
    <ToastProvider>
      <ConfirmProvider>
        <LaunchConfigProvider>
          <AppShell />
        </LaunchConfigProvider>
      </ConfirmProvider>
    </ToastProvider>
  );
}
