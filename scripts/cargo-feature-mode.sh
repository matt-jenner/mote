#!/bin/sh

MOTE_HEIC_MODE=enabled
MOTE_CARGO_FEATURE_ARGS=

mote_disable_heic() {
	MOTE_HEIC_MODE=disabled
	MOTE_CARGO_FEATURE_ARGS='--no-default-features --features mote-defaults'
}

mote_is_invalid_heic_flag() {
	case "$1" in
		--no-heic | --no-heic=* | --no-heicc | --no-hiec | --no-HEIC) return 0 ;;
		*) return 1 ;;
	esac
}
