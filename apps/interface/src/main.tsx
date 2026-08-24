import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { PhotoServiceProvider } from "./app/PhotoServiceContext";
import { AppShell } from "./components/AppShell";
import { createInMemoryPhotoService } from "./services/inMemoryPhotoService";
import "./styles/tokens.css";
import "./styles/global.css";

const queryClient = new QueryClient({
	defaultOptions: {
		queries: { staleTime: Number.POSITIVE_INFINITY, retry: false },
	},
});

const service = createInMemoryPhotoService({ cancelFolderPicker: true });
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
