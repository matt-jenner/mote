import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { PhotoServiceProvider } from "./app/PhotoServiceContext";
import { AppShell } from "./components/AppShell";
import { createHttpPhotoService } from "./services/httpPhotoService";
import { createInMemoryPhotoService } from "./services/inMemoryPhotoService";
import { createTauriPhotoService } from "./services/tauriPhotoService";
import "./styles/tokens.css";
import "./styles/global.css";

const queryClient = new QueryClient({
	defaultOptions: {
		queries: { staleTime: Number.POSITIVE_INFINITY, retry: false },
	},
});

const service =
	import.meta.env.MODE === "memory"
		? createInMemoryPhotoService({ cancelFolderPicker: true })
		: import.meta.env.MODE === "hosted"
			? createHttpPhotoService()
			: createTauriPhotoService();
const root = document.getElementById("root");

if (!root) throw new Error("Application root is missing");

createRoot(root).render(
	<StrictMode>
		<QueryClientProvider client={queryClient}>
			<PhotoServiceProvider service={service}>
				<AppShell />
			</PhotoServiceProvider>
		</QueryClientProvider>
	</StrictMode>,
);
