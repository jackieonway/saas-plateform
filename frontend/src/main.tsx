import React from "react";
import ReactDOM from "react-dom/client";
import { BrowserRouter } from "react-router-dom";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ThemeProvider } from "@/components/theme-provider";
import { Toaster } from "@/components/ui/sonner";
import App from "./App";
import { authStore as sessionStore } from "@/stores/auth";
import { SessionStatusGate } from "@/components/session-status";
import "./index.css";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 30_000,
      retry: 1,
      refetchOnWindowFocus: false,
    },
  },
});

// A context change must synchronously discard all data fetched by the prior
// tenant, including pages that use unscoped legacy query keys.
sessionStore.subscribe((state, previous) => {
  if (state.contextEpoch === previous.contextEpoch) return;
  void queryClient.cancelQueries();
  queryClient.clear();
});

// Register cross-page reconciliation before any private component can mount.
// The gate also keeps unsupported storage environments out of preference code.
void sessionStore.getState().initializeSessionCoordination().catch(() => {
  // The coordinator publishes its unavailable/unknown state to the gate.
});

// Session commits retire page state, while pending navigation keeps the same
// router and history listener after that retirement.
ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <BrowserRouter>
      <SessionStatusGate>
        <QueryClientProvider client={queryClient}>
          <ThemeProvider>
            <App />
            <Toaster position="top-center" richColors closeButton />
          </ThemeProvider>
        </QueryClientProvider>
      </SessionStatusGate>
    </BrowserRouter>
  </React.StrictMode>
);
