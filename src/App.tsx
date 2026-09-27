import { LaunchConfigProvider } from "./features/tasks/LaunchPopover";
import { AppShell } from "./shell/AppShell";
import { ConfirmProvider } from "./ui/ConfirmDialog";
import { Toaster } from "./ui/Sonner";

export default function App() {
  return (
    <>
      <ConfirmProvider>
        <LaunchConfigProvider>
          <AppShell />
        </LaunchConfigProvider>
      </ConfirmProvider>
      <Toaster />
    </>
  );
}
