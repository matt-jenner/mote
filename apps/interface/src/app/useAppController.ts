import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import type {
	Appearance,
	BootstrapState,
	ChooseFolderResult,
	GalleryScope,
} from "../services/photoService";
import { applyAppearance } from "../theme/applyAppearance";
import { usePhotoService } from "./PhotoServiceContext";

const bootstrapKey = ["bootstrap"] as const;

export function useAppController() {
	const service = usePhotoService();
	const queryClient = useQueryClient();
	const acceptFolderSelection = (result: ChooseFolderResult) => {
		if (result.kind === "selected") {
			queryClient.setQueryData<BootstrapState>(bootstrapKey, result.state);
		}
	};
	const bootstrap = useQuery({
		queryKey: bootstrapKey,
		queryFn: () => service.getBootstrapState(),
	});

	useEffect(() => {
		if (bootstrap.data) applyAppearance(bootstrap.data.settings.appearance);
	}, [bootstrap.data]);

	const folder = useMutation({
		mutationFn: () => service.chooseFolder(),
		onSuccess: acceptFolderSelection,
	});
	const appearance = useMutation({
		mutationFn: (value: Appearance) => service.updateAppearance(value),
		onSuccess(state) {
			queryClient.setQueryData<BootstrapState>(bootstrapKey, state);
			applyAppearance(state.settings.appearance);
		},
	});
	const galleryScope = useMutation({
		mutationFn: (value: GalleryScope) => service.updateGalleryScope(value),
		onSuccess(state) {
			queryClient.setQueryData<BootstrapState>(bootstrapKey, state);
		},
	});

	return {
		state: bootstrap.data,
		loading: bootstrap.isPending,
		error:
			bootstrap.error ?? folder.error ?? appearance.error ?? galleryScope.error,
		capabilities: service.capabilities,
		chooseFolder: folder.mutate,
		acceptFolderSelection,
		updateAppearance: appearance.mutate,
		updateGalleryScope: galleryScope.mutate,
	};
}
