set(CMAKE_SYSTEM_NAME Generic)
set(CMAKE_SYSTEM_PROCESSOR arm)
set(CMAKE_TRY_COMPILE_TARGET_TYPE STATIC_LIBRARY)

# Propagate the chosen cross compiler into CMake's nested compiler probes.
set(FIXTURE_C_COMPILER arm-none-eabi-gcc CACHE FILEPATH "ARM-capable C compiler")
list(APPEND CMAKE_TRY_COMPILE_PLATFORM_VARIABLES FIXTURE_C_COMPILER)
set(CMAKE_C_COMPILER "${FIXTURE_C_COMPILER}")

if(FIXTURE_C_COMPILER MATCHES "clang")
    set(CMAKE_C_COMPILER_TARGET arm-none-eabi)
endif()
