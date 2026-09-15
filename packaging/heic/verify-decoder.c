#include <libde265/de265.h>
#include <libheif/heif.h>
#include <stdio.h>
#include <string.h>

int main(int argc, char **argv) {
    if (argc != 3 || strcmp(heif_get_version(), argv[1]) != 0 ||
        strcmp(de265_get_version(), argv[2]) != 0) {
        fputs("Native decoder versions do not match versions.env\n", stderr);
        return 1;
    }
    struct heif_error error = heif_init(NULL);
    if (error.code != heif_error_Ok) {
        fputs("Cannot initialize the native decoder\n", stderr);
        return 1;
    }
    const struct heif_encoder_descriptor *encoders[32];
    const struct heif_decoder_descriptor *decoders[32];
    int encoder_count = heif_get_encoder_descriptors(
        heif_compression_undefined, NULL, encoders, 32);
    int decoder_count = heif_get_decoder_descriptors(
        heif_compression_undefined, decoders, 32);
    int valid = encoder_count == 0 && decoder_count == 1 &&
        strcmp(heif_decoder_descriptor_get_id_name(decoders[0]), "libde265") == 0;
    heif_deinit();
    if (!valid) {
        fprintf(stderr, "Expected only libde265 and zero encoders, found %d decoders and %d encoders\n",
                decoder_count, encoder_count);
        return 1;
    }
    puts("Pinned native decoder: libde265 only, zero encoders");
    return 0;
}
