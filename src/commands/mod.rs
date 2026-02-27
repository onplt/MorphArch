// =============================================================================
// commands/mod.rs — Alt komut modülleri
// =============================================================================
//
// Her CLI alt komutu ayrı bir modülde yaşar:
//   scan    → Git deposu tarama + dependency graph + drift skoru (Sprint 2-3)
//   analyze → Detaylı drift raporu (Sprint 3)
//
// İleride eklenecekler:
//   watch → Dosya izleme + canlı TUI (Sprint 4)
//   diff  → İki commit arası graph karşılaştırma (Sprint 5)
// =============================================================================

pub mod analyze;
pub mod scan;
