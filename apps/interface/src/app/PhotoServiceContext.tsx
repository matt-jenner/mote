import { createContext, type PropsWithChildren, useContext } from "react";
import type { PhotoService } from "../services/photoService";

const PhotoServiceContext = createContext<PhotoService | null>(null);

export function PhotoServiceProvider({
	service,
	children,
}: PropsWithChildren<{ service: PhotoService }>) {
	return (
		<PhotoServiceContext.Provider value={service}>
			{children}
		</PhotoServiceContext.Provider>
	);
}

export function usePhotoService(): PhotoService {
	const service = useContext(PhotoServiceContext);
	if (!service) throw new Error("PhotoServiceProvider is missing");
	return service;
}
