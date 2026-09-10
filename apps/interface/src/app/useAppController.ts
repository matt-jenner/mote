import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { folderLabel, sourceIsUnavailable } from "../folders/savedFolders";
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
		refetchOnWindowFocus: false,
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

	useEffect(
		() =>
			service.watchSavedFolders((savedFolders) => {
				queryClient.setQueryData<BootstrapState>(bootstrapKey, (previous) => {
					if (!previous) return previous;
					let activeSource = previous.activeSource;
					if (savedFolders.activeEntryId === null) activeSource = null;
					const entry = savedFolders.entries.find(
						(e) => e.id === savedFolders.activeEntryId,
					);
					if (
						entry &&
						activeSource &&
						savedFolders.activeEntryId === previous.savedFolders.activeEntryId
					) {
						const access = savedFolders.access[entry.folderId]?.state;
						activeSource = {
							...activeSource,
							displayName: folderLabel(entry),
							availability:
								access && sourceIsUnavailable(access)
									? access === "unverified"
										? "rootOffline"
										: (access as "missing" | "unreadable" | "rootOffline")
									: access === "available"
										? "available"
										: activeSource.availability,
						};
					}
					return { ...previous, activeSource, savedFolders };
				});
			}),
		[service, queryClient],
	);
	const entryIds =
		bootstrap.data?.savedFolders.entries.map((e) => e.id).join(",") ?? "";
	useEffect(() => {
		const check = () => {
			if (document.visibilityState !== "hidden")
				void service
					.checkSavedFolders(entryIds ? entryIds.split(",") : [])
					.catch(() => {});
		};
		check();
		window.addEventListener("focus", check);
		document.addEventListener("visibilitychange", check);
		return () => {
			window.removeEventListener("focus", check);
			document.removeEventListener("visibilitychange", check);
		};
	}, [service, entryIds]);
	const accept = (state: BootstrapState) => {
		queryClient.setQueryData<BootstrapState>(bootstrapKey, state);
	};

	return {
		state: bootstrap.data,
		loading: bootstrap.isPending,
		error:
			bootstrap.error ?? folder.error ?? appearance.error ?? galleryScope.error,
		capabilities: service.capabilities,
		chooseFolder: folder.mutate,
		activateSavedFolder: async (id: string) => {
			const result = await service.activateSavedFolder(id);
			acceptFolderSelection(result);
			return result.kind === "selected";
		},
		renameSavedFolder: async (id: string, label: string) => {
			accept(await service.renameSavedFolder(id, label));
		},
		removeSavedFolder: async (id: string) => {
			accept(await service.removeSavedFolder(id));
		},
		acceptFolderSelection,
		updateAppearance: appearance.mutate,
		updateGalleryScope: galleryScope.mutate,
	};
}
