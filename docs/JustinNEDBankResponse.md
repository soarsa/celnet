Hi Ben and Adrian,
 
Sure, happy to do that.. I just want to give you some more detail on what I would be looking for. Also working on the pricing tools to make it simpler for clients and sales people to price/model and execute.
 
Product: seems all are covered, just wanted clarity on:
Asians, we need Arithmetic average rate, not only Geometric, ie not closed form solution, would need lognormal approx. or monte carlo.. something along those lines
Option on Futures – black76, black at forward price type – I think that the product would just need to be catered for, simple I think with this stack you have presented. It would need to reflect that it’s a option on future in the STP, so booking would be option on a future instrument in the back end to FAFX as is reconciled against JSE for margin purposes etc (could even be that when the trade hits FAFX its pushed to Celer Options) sorts out 2 way recon.
Pricers with solvers for various factors, spot/vega/delta/strike/expiry/premium/across multiple strategies
support for FX futures
TARF’s but not too important at the moment
 
Risk Management: not exhaustive but top of mind
Wanted to explore at the risk management reporting in a little more detail. As I see that the spot/vol/rates/time shifts seem to be part of the reporting
Wanted to explore the 2 factor reports spot and vol on grid
Other operational reporting
Pnl attributions per risk category daily pnl/Attributed pnl(Spot/vol(smile)/rates/time/amendments/events/new trade)
Vega risk per vol curve attributed (ATM, 25d RR, 10d RR, 25d fly and 10d fly) / tenor pillar  (vega/volga/vanna)
Rates risk per tenor per ccy
Strike topography
Option on Future/NDO positions vs cash delivered option, as we will need to replace delta on fixings in NDO/Future space vs total portfolio risk management.
Sales margin capture and reporting – different portfolios
Seeing cash balances for rolling balance/settlement diary/reporting
Expiry reporting with greeks, vega/theta/gamma most important
 
Market data
spot and forwards I assume will be coming from the celer (spot/forwards/ndf) stack currently being implemented
USD SOFR/depo curve – From FAFX I presume
vol curves
sourced pairs eg EURUSD/GBPUSD/USDJPY from sources like spectraxe/digivega/bloomberg/other
independent curves
how do you envisage this to work? Base curves, correlation or driven pairs.
spread management, vanilla space and exotic space (how do we spread options products)
event management.. NFP/CPI/FED/MPC events etc and their impacts per pair
 
EOD processes.
Locking market data for EOD flash reporting – all systems need to reval off same market data.
Starting and stopping market data for risk reporting.
While allowing pricing to continue on live market data
Any other processes that needs to take place
 
MI – nice to haves
Historical data for back dated scenario tooling
Seeing what clients are pricing/RFQ’ing for sales/trading to view
 
I hope that this should cover most of the bases, so we just need to work through this and the additional attributes of your pdf.


Justin NedBank.  
