#include "text_service.h"

#include <algorithm>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <iostream>

namespace owo::tsf {
struct CandidateRenderPreviewHarness {
    static bool initialize(TextService& service) {
        return SUCCEEDED(service.initialize_windows());
    }
    static bool snapshot(TextService& service, const std::filesystem::path& path,
                         const bool expanded, const bool hover, const bool glass) {
        service.input_buffer_ = L"xiufuhouxuanyanmiandonghua";
        service.segmented_input_ = L"xiu'fu'hou'xuan'yan'mian'dong'hua";
        service.input_cursor_ = service.input_buffer_.size();
        service.candidates_ = {L"修复候选界面动画", L"修复", L"候选界面", L"修", L"秀", L"休", L"宿"};
        service.candidate_groups_ = {0, 1, 1, 2, 2, 2, 2};
        if (expanded) {
            const auto first = service.candidates_;
            const auto groups = service.candidate_groups_;
            for (int row = 1; row < 5; ++row) {
                service.candidates_.insert(service.candidates_.end(), first.begin(), first.end());
                service.candidate_groups_.insert(service.candidate_groups_.end(), groups.begin(), groups.end());
            }
        }
        service.candidates_expanded_ = expanded;
        service.candidate_page_size_ = 7;
        service.has_more_candidates_ = !expanded;
        service.candidate_anchor_valid_ = true;
        service.candidate_motion_enabled_ = true;
        service.hovered_target_.reset();
        service.pressed_target_.reset();
        service.candidate_interactions_.clear();
        service.candidate_disclosure_motion_.reset(expanded ? 1.0F : 0.0F);
        service.candidate_emphasis_motion_.reset(1.0F);
        if (hover) {
            service.hovered_target_ = TextService::HitTarget{TextService::HitKind::candidate, 1};
            TextService::CandidateInteractionMotion motion{
                *service.hovered_target_, CandidateMotion{}, CandidateMotion{}};
            motion.hover.reset(1.0F);
            service.candidate_interactions_.push_back(motion);
        }
        service.candidate_theme_ = {};
        if (glass) {
            service.candidate_theme_.background = {0x101827, 0.82F};
            service.candidate_theme_.border = {0x88AACC, 0.35F};
            service.candidate_theme_.pinyin_text = {0x9CCBFF, 1.0F};
            service.candidate_theme_.background_blur_enabled = true;
        }
        service.discard_device_resources();
        service.candidate_content_size_ = service.desired_candidate_window_size();
        service.candidate_present_size_ = service.candidate_content_size_;
        service.candidate_present_geometry_valid_ = true;
        const auto width = service.candidate_present_size_.cx;
        const auto height = service.candidate_present_size_.cy;
        // Inject a deterministic synthetic backdrop: no screen capture, even
        // when exercising the plugin-compatible glass compositing path.
        service.blurred_candidate_background_.assign(
            static_cast<std::size_t>(width) * height, 0xff465970U);
        if (!service.render_candidate_window()) return false;
        const auto visible = std::count_if(service.hit_regions_.begin(), service.hit_regions_.end(),
            [](const auto& region) { return region.target.kind == TextService::HitKind::candidate; });
        if (visible != (expanded ? 35 : 7) || service.candidate_motion_timer_running_) return false;
        for (const auto& hit : service.hit_regions_) {
            if (hit.bounds.left < 0 || hit.bounds.top < 0 || hit.bounds.right > width ||
                hit.bounds.bottom > height) return false;
        }
        BITMAPV4HEADER header{};
        header.bV4Size = sizeof(header);
        header.bV4Width = width;
        header.bV4Height = -height;
        header.bV4Planes = 1;
        header.bV4BitCount = 32;
        header.bV4V4Compression = BI_BITFIELDS;
        header.bV4RedMask = 0x00ff0000;
        header.bV4GreenMask = 0x0000ff00;
        header.bV4BlueMask = 0x000000ff;
        header.bV4AlphaMask = 0xff000000;
        header.bV4CSType = LCS_sRGB;
        header.bV4SizeImage = static_cast<DWORD>(width * height * 4);
        BITMAPFILEHEADER file{};
        file.bfType = 0x4d42;
        file.bfOffBits = sizeof(file) + sizeof(header);
        file.bfSize = file.bfOffBits + header.bV4SizeImage;
        std::ofstream output(path, std::ios::binary | std::ios::trunc);
        output.write(reinterpret_cast<const char*>(&file), sizeof(file));
        output.write(reinterpret_cast<const char*>(&header), sizeof(header));
        output.write(static_cast<const char*>(service.layered_surface_pixels_), header.bV4SizeImage);
        return static_cast<bool>(output);
    }
};
}

int wmain(int argc, wchar_t** argv) {
    if (argc != 2) return 2;
    SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    const auto initialized = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
    if (FAILED(initialized)) return 3;
    auto* service = new owo::tsf::TextService;
    bool valid = owo::tsf::CandidateRenderPreviewHarness::initialize(*service);
    const std::filesystem::path directory(argv[1]);
    std::filesystem::create_directories(directory);
    if (valid) {
        valid = owo::tsf::CandidateRenderPreviewHarness::snapshot(*service, directory / L"candidate-default.bmp", false, false, false) &&
            owo::tsf::CandidateRenderPreviewHarness::snapshot(*service, directory / L"candidate-hover.bmp", false, true, false) &&
            owo::tsf::CandidateRenderPreviewHarness::snapshot(*service, directory / L"candidate-expanded.bmp", true, true, false) &&
            owo::tsf::CandidateRenderPreviewHarness::snapshot(*service, directory / L"candidate-glass.bmp", false, false, true);
    }
    service->Release();
    CoUninitialize();
    std::cout << (valid ? "Synthetic candidate rendering and hit-region checks passed\n"
                       : "Synthetic candidate rendering failed\n");
    return valid ? 0 : 1;
}
