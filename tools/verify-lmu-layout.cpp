// Optional SDK compatibility check, not part of the Rust build. Compile with
// Windows x64 C++ and /I pointing to the game's Support/SharedMemoryInterface.
// No proprietary header is included in the repository or modified by this tool.
#include <stdint.h>
#include <stddef.h>
#include <cstring>
#include <utility>
#include <windows.h>
#include "SharedMemoryInterface.hpp"

static_assert(sizeof(long) == 4 && sizeof(void*) == 8 && sizeof(bool) == 1,
              "Use the Windows x64 ABI");
static_assert(sizeof(ScoringInfoV01) == 548);
static_assert(sizeof(VehicleScoringInfoV01) == 584);
static_assert(sizeof(TelemInfoV01) == 1888);
static_assert(sizeof(SharedMemoryObjectOut) == 324824);
static_assert(sizeof(SharedMemoryLayout) == 324824);
static_assert(offsetof(SharedMemoryGeneric, gameVersion) == 64);
static_assert(offsetof(SharedMemoryObjectOut, scoring) == 1632);
static_assert(offsetof(ScoringInfoV01, mSession) == 64);
static_assert(offsetof(ScoringInfoV01, mInRealtime) == 115);
static_assert(offsetof(SharedMemoryObjectOut, telemetry) == 128464);
static_assert(offsetof(SharedMemoryTelemetryData, telemInfo) == 4);
static_assert(offsetof(TelemInfoV01, mID) == 0);
static_assert(offsetof(TelemInfoV01, mElapsedTime) == 12);
static_assert(offsetof(TelemInfoV01, mVehicleName) == 32);
static_assert(offsetof(TelemInfoV01, mLocalVel) == 184);
static_assert(offsetof(TelemInfoV01, mGear) == 352);
static_assert(offsetof(TelemInfoV01, mEngineRPM) == 356);
static_assert(offsetof(TelemInfoV01, mUnfilteredThrottle) == 388);
static_assert(offsetof(TelemInfoV01, mUnfilteredBrake) == 396);
static_assert(offsetof(TelemInfoV01, mEngineMaxRPM) == 532);
