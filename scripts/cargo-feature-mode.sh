#!/bin/sh

MOTE_HEIC_MODE=enabled
MOTE_CARGO_FEATURE_ARGS=

mote_disable_heic() {
	MOTE_HEIC_MODE=disabled
	MOTE_CARGO_FEATURE_ARGS='--no-default-features --features mote-defaults'
}
