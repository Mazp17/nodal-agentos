import { AppShell } from "./shell/AppShell";
import { ConfirmProvider } from "./ui/ConfirmDialog";
import { Toaster } from "./ui/Sonner";

export default function App() {
  return (
    <>
      <ConfirmProvider>
        <AppShell />
      </ConfirmProvider>
      <Toaster />
    </>
  );
}
