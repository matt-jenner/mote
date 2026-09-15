# The pinned libheif release registers its mask encoder unconditionally.
# Apply only these exact source removals, after archive verification and before
# configuration. Refuse source drift rather than silently retaining an encoder.
function(remove_exact file needle)
    file(READ "${SOURCE_DIR}/${file}" source)
    string(FIND "${source}" "${needle}" position)
    if(position EQUAL -1)
        message(FATAL_ERROR "Decode-only patch no longer matches ${file}")
    endif()
    string(REPLACE "${needle}" "" source "${source}")
    file(WRITE "${SOURCE_DIR}/${file}" "${source}")
endfunction()

remove_exact("libheif/plugin_registry.cc" "#include \"plugins/encoder_mask.h\"\n")
remove_exact("libheif/plugin_registry.cc" "  register_encoder(get_encoder_plugin_mask());\n")
remove_exact("libheif/plugins/CMakeLists.txt" "               encoder_mask.h\n")
remove_exact("libheif/plugins/CMakeLists.txt" "               encoder_mask.cc\n")
