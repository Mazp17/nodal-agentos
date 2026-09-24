import { AppShell } from "./shell/AppShell";
import { ToastProvider } from "./ui/Toasts";

export default function App() {
  return (
    <ToastProvider>
      <AppShell />
    </ToastProvider>
  );
}
